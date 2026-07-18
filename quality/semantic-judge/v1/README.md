# Semantic judge v1 — generic executable contract

**Status:** Frozen by T012 (2026-07-17). Formal JSON Schema files are T023.

Related documents:

- [Development policy](../../../docs/development-policy.md)
- [Decision D012](../../../docs/change/initial-implementation/decisions.md#d012--semantic-judge-provisioning)
- [Foundation seed rubric manifest](../../rubrics/manifest.json)

## Purpose

Development tooling invokes replaceable semantic judges through one versioned generic executable contract. The product runtime has no dependency on this executable.

## Process model

1. Caller writes one UTF-8 JSON request document to the judge executable stdin and closes stdin.
2. Judge reads stdin until EOF, evaluates the request, writes one UTF-8 JSON result document to stdout, and exits.
3. Malformed request, adapter failure, timeout, or malformed model response map to `verdict: unavailable` on stdout (not a non-zero adapter exit solely for unavailable).

## Request v1

| Field | Type | Required | Rule |
|---|---|---:|---|
| `schema_version` | integer | yes | Must be `1` |
| `mode` | string | yes | `local` or `publication` |
| `parent_revision` | string | yes | Parent git object name |
| `candidate_revision` | string | yes | Candidate git object name |
| `diff` | string | yes | Exact parent-to-candidate diff text |
| `relevant_docs` | array | no | `{path, content}` resulting-doc snapshots |
| `rubrics` | array | yes | Non-empty `{id, content}` parent rubric payloads |
| `deterministic_evidence` | array | yes | `{command, exit_code, stdout, stderr}` |
| `timeout_seconds` | integer | no | Overrides configured default |

Before focused rubric files exist (through T024), callers load the foundation seed rubric from the parent revision's committed `quality/rubrics/manifest.json` when present. When that manifest is absent at the parent revision, callers compose the foundation-seed rubric only from immutable Git blobs at foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` via `git show <sha>:<approved source path>` for I47 (`docs/invariants.md`), Git enforcement direction (`docs/testing.md`), tenet 27 (`docs/tenets.md`), and composition/enforcement (`docs/architecture.md`). Source paths and blob digests are frozen in `request_builder.py`; composed rubric content must match digest `3f1bd3489401ca6114ac1ef756ad4e87798a2d1ed3973c16625fd87167c1b3cd`. Request `deterministic_evidence` must expose per-source provenance. The bundled file at `quality/semantic-judge/v1/frozen/foundation-seed.v1.md` is a non-authoritative reference only. Callers must never read rubric authority from the candidate working tree. T025 focused rubrics apply only to the commit following T025.

## Response v1

| Field | Type | Required | Rule |
|---|---|---:|---|
| `schema_version` | integer | yes | Must be `1` |
| `parent_revision` | string or null | yes | Echo request `parent_revision` when both request revisions are valid non-empty strings; otherwise `null` |
| `candidate_revision` | string or null | yes | Echo request `candidate_revision` when both request revisions are valid non-empty strings; otherwise `null` |
| `verdict` | string | yes | `pass`, `fail`, `indeterminate`, or `unavailable` |
| `citations` | array | yes | Empty only for `unavailable` |
| `message` | string | yes | Human-readable rationale |

Every response binds to the judged revision pair atomically: when the parsed request provides both valid non-empty revision strings, every later `unavailable` path (including config read/parse errors) echoes both revisions; otherwise both revision fields are `null`. The adapter rejects model output whose `parent_revision` or `candidate_revision` does not exactly match the request and maps the result to `unavailable` while echoing the request revisions. Malformed stdin before JSON extraction emits both revision fields as `null`.

Citation object:

| Field | Type | Required |
|---|---|---:|
| `rubric_id` | string | yes |
| `rule` | string | yes |
| `lines` | string array | yes |

`pass`, `fail`, and `indeterminate` require at least one citation. Judges must cite parent rubric rules and changed/resulting lines rather than invent build/test claims.

## Dispositions

| Mode | `pass` | `fail` | `indeterminate` | `unavailable` |
|---|---|---|---|---|
| `local` | allow commit | block commit | warn; allow commit | warn; allow commit |
| `publication` | allow publication | block publication | block publication | block publication |

Bootstrap publication exception was consumed by foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`. No later commit receives a bootstrap exception.

## Provisioned local adapter (T012)

| Artifact | Path |
|---|---|
| Executable | `quality/semantic-judge/v1/judge` |
| Adapter implementation | `quality/semantic-judge/v1/adapter.py` |
| Adapter config | `quality/semantic-judge/v1/config.json` |
| Default model | `openai-codex/gpt-5.6-sol` via `pi` with `--no-tools` |
| Default timeout | 900 seconds |

Credentials are never stored in the repository. Local execution uses the operator's existing `pi` authentication (for example `~/.pi/agent/auth.json`). T029 provisions the same adapter in GitHub Actions using repository secret `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON`.

## Verification

```bash
quality/semantic-judge/v1/verify-contract
quality/semantic-judge/v1/build-exact-staged-request | quality/semantic-judge/v1/judge
quality/semantic-judge/v1/build-smoke-request | quality/semantic-judge/v1/judge
```

`build-exact-staged-request` is the generic C0 local gate: parent `HEAD`, candidate index tree, staged diff/resulting docs only. `build-smoke-request` is T012-only publication smoke for parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` and candidate `581c23bb085718d37d994d77d59d3d70b7ea309f`.

Protocol fixture matrix lives under `fixtures/response-*.v1.json`. Fixtures document response shapes only; they are not publication authority.
