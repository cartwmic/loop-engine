#!/usr/bin/env python3
"""Generic semantic judge v1 executable adapter (pi / openai-codex)."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
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

    required = (
        "schema_version",
        "mode",
        "parent_revision",
        "candidate_revision",
        "diff",
        "rubrics",
        "deterministic_evidence",
    )
    for key in required:
        if key not in request:
            reject(f"missing request field: {key}")
    if request.get("schema_version") != 1:
        reject("unsupported request schema_version")
    if request.get("mode") not in {"local", "publication"}:
        reject("mode must be local or publication")
    if not isinstance(request.get("parent_revision"), str) or not request["parent_revision"]:
        reject("parent_revision must be a non-empty string")
    if not isinstance(request.get("candidate_revision"), str) or not request["candidate_revision"]:
        reject("candidate_revision must be a non-empty string")
    if not isinstance(request.get("rubrics"), list) or not request["rubrics"]:
        reject("rubrics must be a non-empty array")
    if not isinstance(request.get("deterministic_evidence"), list):
        reject("deterministic_evidence must be an array")


def validate_response(payload: dict[str, Any]) -> str | None:
    if payload.get("schema_version") != 1:
        return "unsupported response schema_version"
    verdict = payload.get("verdict")
    if verdict not in VALID_VERDICTS:
        return "invalid verdict"
    parent_revision = payload.get("parent_revision")
    candidate_revision = payload.get("candidate_revision")
    if parent_revision is None or candidate_revision is None:
        if parent_revision is not None or candidate_revision is not None:
            return "parent_revision and candidate_revision must both be present or both null"
        if verdict != "unavailable":
            return "missing response field: parent_revision"
    else:
        if not isinstance(parent_revision, str) or not parent_revision:
            return "parent_revision must be a non-empty string"
        if not isinstance(candidate_revision, str) or not candidate_revision:
            return "candidate_revision must be a non-empty string"
    citations = payload.get("citations")
    if not isinstance(citations, list):
        return "citations must be an array"
    if verdict == "unavailable":
        if citations:
            return "unavailable verdict must have empty citations"
        if not isinstance(payload.get("message"), str) or not payload["message"]:
            return "unavailable verdict requires message"
        return None
    if not isinstance(payload.get("message"), str) or not payload["message"]:
        return "determinate verdict requires message"
    if not citations:
        return "pass/fail/indeterminate require at least one citation"
    for citation in citations:
        if not isinstance(citation, dict):
            return "citation must be object"
        for key in ("rubric_id", "rule", "lines"):
            if key not in citation:
                return f"citation missing {key}"
        if not isinstance(citation["lines"], list) or not citation["lines"]:
            return "citation lines must be non-empty array"
    return None


def extract_json_object(text: str) -> dict[str, Any] | None:
    text = text.strip()
    if not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass
    match = re.search(r"\{.*\}", text, flags=re.DOTALL)
    if not match:
        return None
    try:
        return json.loads(match.group(0))
    except json.JSONDecodeError:
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

    return f"""You are a documentation semantic judge for loop-engine.

Evaluate whether the candidate revision is documentation-coherent under the parent revision's rubric.

Rules:
- Use ONLY the parent rubric text provided below.
- Judge ONLY the exact parent-to-candidate diff and resulting relevant docs.
- Use deterministic evidence for build/test/check claims; never invent CI or test results.
- Emit exactly one JSON object and no other text.
- verdict must be one of: pass, fail, indeterminate.
- pass/fail/indeterminate require at least one citation with rubric_id, rule, and non-empty lines array.
- lines entries must cite repository paths with optional :line, e.g. docs/testing.md:220.
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

## Exact diff

```diff
{request['diff']}
```

Respond with JSON only using this shape:
{{"schema_version":1,"parent_revision":"{request['parent_revision']}","candidate_revision":"{request['candidate_revision']}","verdict":"pass|fail|indeterminate","citations":[{{"rubric_id":"...","rule":"...","lines":["path:line"]}}],"message":"..."}}

parent_revision and candidate_revision in the response must exactly match the request values above.
"""


def invoke_pi(
    prompt: str,
    config: dict[str, Any],
    timeout_seconds: int,
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
    if completed.returncode != 0 and not completed.stdout.strip():
        stderr = (completed.stderr or "").strip()
        unavailable(
            f"pi exited {completed.returncode}: {stderr or 'no stdout'}",
            binding=binding,
        )
    text = extract_pi_assistant_text(completed.stdout)
    if not text:
        unavailable(
            "no assistant response from pi",
            binding=binding,
        )
    return text


def judge(request: dict[str, Any], binding: RevisionBinding) -> dict[str, Any]:
    validate_request(request, binding)
    parent_revision = request["parent_revision"]
    candidate_revision = request["candidate_revision"]
    config = load_config(binding)
    timeout_seconds = int(request.get("timeout_seconds") or config.get("timeout_seconds") or 300)
    prompt = build_prompt(request)
    raw = invoke_pi(
        prompt,
        config,
        timeout_seconds,
        binding=binding,
    )
    payload = extract_json_object(raw)
    if payload is None:
        unavailable(
            "judge returned non-json response",
            binding=binding,
        )
    error = validate_response(payload)
    if error:
        unavailable(
            f"invalid judge response: {error}",
            binding=binding,
        )
    if (
        payload.get("parent_revision") != parent_revision
        or payload.get("candidate_revision") != candidate_revision
    ):
        unavailable(
            "response revision binding mismatch",
            binding=binding,
        )
    return payload


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
