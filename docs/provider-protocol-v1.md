# Provider Protocol v1 — Transport

**Status:** Frozen by T005 (2026-07-17). Decision [D005](change/initial-implementation/decisions.md#d005--provider-protocol-v1).

This document is the language-neutral normative contract for provider subprocess transport. An independent provider author can implement protocol v1 from this document without reading engine source. JSON Schema files and published examples are planned in T084/T192; this document defines envelopes, process behavior, role semantics, and outcome mapping.

Related documents:

- [Decision D005](change/initial-implementation/decisions.md#d005--provider-protocol-v1)
- [Decision D008](change/initial-implementation/decisions.md#d008--resource-bounds-and-timeout-defaults) — authoritative numeric bounds in [cli-contract.md](cli-contract.md#resource-bounds-d008)
- [Application operation catalog](operation-catalog.md) — provider role/result rows and reason taxonomy (D004)
- [Product intent](intent.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [Graph projection and canonical encoding](graph-projection.md)
- [System invariants](invariants.md)
- [Interaction storyboards](ux-storyboards.md)

## Scope

Protocol v1 covers:

- one fresh provider OS process per protocol invocation;
- exact process command, working directory, environment, and timeout behavior;
- stdin/stdout/stderr byte framing;
- `protocol_major` negotiation;
- five tagged roles and role-specific result applicability;
- immutable resolved-registration handoff (invoker never queries catalog during invocation);
- inherited-environment policy with no registration overrides and no environment in trace payloads;
- same-major unknown-field policy;
- dedicated process-group spawn with PGID verification and caller-isolated timeout termination on supported Unix platforms;
- no provider state authority.

Out of scope for v1 (stop conditions — redesign before proceeding):

- persistent provider processes or connection reuse;
- streaming, chunked, or multiplexed protocol on one stdio channel;
- Windows provider subprocess transport.

Graph projection field semantics and canonical encoding are frozen in [graph-projection.md](graph-projection.md) (T014, D014). Evidence wire shapes are frozen separately (T084). This document references graph projection only where transport requires shape or bounds.

## Process model

Each provider role invocation uses **one fresh OS process**:

1. Engine integration resolves the current registration **exactly once** through the provider-catalog capability and constructs an immutable `registration` object (see [Resolved registration handoff](#resolved-registration-handoff)).
2. Integration establishes a dedicated provider process group, spawns the provider with resolved argv, working directory, and inherited environment, and verifies provider PGID (see [Process group establishment](#process-group-establishment)).
3. Integration writes one UTF-8 JSON request document to the process stdin and closes stdin (EOF).
4. Provider reads stdin until EOF, parses one request, writes one UTF-8 JSON result document to stdout, and exits.
5. Integration reads stdout until EOF, parses one result, records exit status/signals, and maps transport outcome.

There is no session identifier across invocations, no stdio keep-alive, and no engine-side reuse of a provider process for a later role call. Retries are not automatic (I44).

## Process invocation

Integration launches the provider with:

| Parameter | Source | Rule |
|---|---|---|
| `argv[0]` (executable) | `registration.executable` | Absolute path supplied by integration/OS as **argv[0]** to the `exec`-family syscall. No shell, no implicit `/bin/sh -c`, no word splitting, no variable expansion. |
| `argv[1..]` | `registration.argv` | Arguments after `argv[0]`, passed verbatim. May be empty (`[]`). Integration never inserts additional argv elements. |
| Working directory | `registration.working_directory` | Absolute directory path. **Not** the CLI caller's current working directory. |
| Environment | Engine process environment | Complete inheritance unchanged. Registration stores **no** environment overrides. |
| Timeout | `registration.timeout_seconds` | Wall-clock limit for the invocation; see [Timeout and termination](#timeout-and-termination). |

A provider may be a shebang script referenced by `registration.executable`. The engine still performs literal execution of that path as `argv[0]` with `registration.argv` as `argv[1..]`.

## Process group establishment

On supported Unix platforms (D002), before transferring control to the provider executable:

1. Integration creates the provider process in a **new dedicated process group** isolated from the engine (caller) process group.
2. After spawn, integration **verifies** that the provider process belongs to the expected dedicated process group (PGID).
3. Timeout termination signals are sent **only** to that verified provider PGID. Integration must never signal the caller's process group for provider timeout handling.
4. If process-group establishment or PGID verification fails before the provider is considered running, integration aborts the invocation as `provider.spawn.failed` and must not have signaled the caller group for provider timeout purposes.

Provider code runs with the caller's OS permissions. The engine does not sandbox providers (I40).

## Byte framing

### Stdin (request)

- Content: exactly **one** UTF-8 JSON object (the request envelope).
- After the JSON document bytes, integration closes stdin so the provider observes EOF.
- No length prefix, delimiter line, or trailing bytes after the JSON document.
- Maximum encoded request size: **`provider_request_json_bytes`** (4 MiB; [cli-contract.md](cli-contract.md#resource-bounds-d008)).
- If encoded request size exceeds the bound **before spawn**, integration rejects the invocation as an engine-side operation error (`resource.exhausted`); the provider is never spawned and never receives the request.
- Invalid UTF-8 in the encoded request document (validated before spawn) is `provider.protocol.invalid_utf8`.

### Stdout (result)

- Content: exactly **one** UTF-8 JSON object (the result envelope).
- Provider writes the document then closes stdout and exits.
- Integration drains stdout until the provider process exits (or stdout closes).
- Maximum encoded result size: **`provider_result_stdout_bytes`** (1 MiB; [cli-contract.md](cli-contract.md#resource-bounds-d008)).
- If encoded stdout exceeds the bound, the invocation maps to `provider.protocol.oversized`.
- Integration must drain the full stdout stream regardless of size. Operational trace retains in-bound stdout exactly; when retention exceeds the bound, integration stores a prefix up to the bound and records explicit truncation metadata (original byte length and truncated flag).
- Empty stdout, multiple JSON values, or trailing non-whitespace after the document is `provider.protocol.malformed`.
- Invalid UTF-8 in the stdout result document is `provider.protocol.invalid_utf8`.

### Stderr (diagnostics)

- Arbitrary diagnostic byte stream. It does not carry the authoritative result and is not required to be UTF-8.
- Integration drains stderr until the provider process exits.
- Maximum retained size in operational trace: **`provider_stderr_trace_bytes`** (1 MiB; [cli-contract.md](cli-contract.md#resource-bounds-d008)).
- When retention exceeds the bound, integration stores a prefix up to the bound and records explicit truncation metadata (original byte length and truncated flag) in the operational trace.
- Stderr size or retention limits do **not** map to `provider.protocol.oversized`.
- Stderr content never overrides semantic mapping of a valid, independently complete stdout result; non-zero exit after valid stdout still yields `provider.nonzero_exit` unless a more specific signal/crash code applies.

### Exit status

- **Exit 0** after a valid stdout result received before the wall-clock deadline is the normal success path.
- **Non-zero exit** after any stdout observation maps to `provider.nonzero_exit`.
- **Signal termination** maps to `provider.signal` or `provider.crash` per platform semantics.
- **Timeout** maps to `provider.timeout` unconditionally once the wall-clock deadline fires; see [Timeout and termination](#timeout-and-termination).

## Version negotiation

- `protocol_major` is a positive integer carried in both request and result envelopes.
- MVP supports **`protocol_major` = 1** only.
- Unsupported `protocol_major` in the request or result envelope is a **transport/protocol operation error** (`provider.protocol.unsupported_major`) for **every** role. Integration maps unsupported-major failures at the transport layer; it never routes this condition through role-specific `result.kind` values.
- `describe` has no role-valid denial or `evaluation_error` result. Unsupported-major handling is therefore role-neutral and never uses `evaluation_error`.
- Request with unsupported `protocol_major` must not be silently accepted. Provider may write a minimal result envelope (echoing `invocation_id` and `role`) when possible; integration still maps the invocation to `provider.protocol.unsupported_major` regardless of any `result.kind` present.
- Result envelope with `protocol_major` other than the supported major is `provider.protocol.unsupported_major`.
- Malformed, unreadable, or missing stdout after an unsupported-major request is also `provider.protocol.unsupported_major`.
- Within major **1**, same-major evolution is additive: new optional fields may appear; receivers ignore unknown fields.

## Request envelope

Every request is one JSON object:

```json
{
  "protocol_major": 1,
  "role": "<role-name>",
  "invocation_id": "<opaque-correlation-string>",
  "registration": { },
  "payload": { }
}
```

| Field | Required | Description |
|---|---|---|
| `protocol_major` | yes | Must be `1` for v1. |
| `role` | yes | One of the five roles in [Five roles](#five-roles). |
| `invocation_id` | yes | Opaque string correlating request, result, and operational trace. Integration generates it; provider echoes it unchanged in the result. |
| `registration` | yes | Immutable resolved registration snapshot; see below. |
| `payload` | yes | Role-specific input object. Use `{}` when a role has no inputs. |

The protocol defines **no** catalog-query message, registration lookup, or follow-up round trip. The provider receives everything integration chose to pass in this single request.

## Result envelope

Every successful protocol exchange returns one JSON object on stdout:

```json
{
  "protocol_major": 1,
  "role": "<same-as-request>",
  "invocation_id": "<echo-request>",
  "provider_version": "<optional-audit-string>",
  "result": { "kind": "<role-specific>" }
}
```

| Field | Required | Description |
|---|---|---|
| `protocol_major` | yes | Must be `1` for v1. |
| `role` | yes | Must match request `role`. |
| `invocation_id` | yes | Must exactly match request `invocation_id`. |
| `provider_version` | no | Optional audit-only self-report of the provider build or implementation version. Never affects protocol semantics. When present, integration preserves it as available invocation audit data in operational trace and in durable journal/provider-observation facts whenever the operation can durably record the invocation (I8, I43). |
| `result` | yes | Tagged union discriminated by `result.kind`. Only role-valid kinds are permitted. |

Mismatch of `role` or `invocation_id` is `provider.protocol.malformed`.

## Resolved registration handoff

Integration resolves the enabled registration **once** per engine operation need, then passes an immutable snapshot on every invocation in that resolution scope. The provider process has no API to query the catalog, resolve handles, or refresh registration.

```json
{
  "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
  "config_revision": 1,
  "executable": "/opt/providers/workflow",
  "argv": [],
  "working_directory": "/opt/providers",
  "timeout_seconds": 60
}
```

| Field | Description |
|---|---|
| `registration_id` | Stable immutable machine-local workflow identity (I37). |
| `config_revision` | Monotonic registration configuration revision observed at resolve time (D009). |
| `executable` | Absolute executable path used as `argv[0]` at spawn. |
| `argv` | Arguments after `argv[0]` (`argv[1..]`), passed verbatim. Empty list is valid. |
| `working_directory` | Absolute working directory used for spawn. |
| `timeout_seconds` | Positive integer wall-clock timeout for this invocation. Default **`provider_timeout_seconds_default`** ([cli-contract.md](cli-contract.md#resource-bounds-d008)). |

Between `describe` and `validate_inputs` during `run.create`, integration uses the same resolved `registration_id`, `config_revision`, `executable`, `argv`, and `working_directory`, and compares observed executable digest when available. Detected executable change without a matching config revision is `provider.drift.detected` (operation error).

## Environment policy

- Provider inherits the engine process environment **verbatim**.
- Registration records store **no** environment-variable overrides and protocol messages carry **no** environment block.
- Operational trace records bounded protocol payloads and streams; it **never** copies the inherited environment into trace events (I42, I46).
- Providers that need secrets or tool paths must read them from the inherited environment under their own conventions; the engine does not inject or redact them.

## Unknown-field policy

Within `protocol_major` 1:

- Receivers **must** ignore JSON object fields with unrecognized names.
- Senders **may** include additional optional fields without incrementing `protocol_major`.
- Senders **must not** rely on receivers understanding new required semantics without a new major version.
- Unknown `result.kind` values are `provider.protocol.malformed`.

## Timeout and termination

On supported Unix platforms (D002), when `timeout_seconds` elapses:

1. Integration sends **SIGTERM** to the **verified provider process group** only (see [Process group establishment](#process-group-establishment)).
2. After a **5-second** grace period, integration sends **SIGKILL** to the same verified provider PGID if any member remains.
3. Orphaned child processes must not survive timeout.

Once the wall-clock deadline fires, the invocation outcome is **unconditionally** `provider.timeout`. Partial or complete stdout observed after the deadline does not change this mapping. A successful invocation requires that a complete valid result envelope **and** process exit (per [Exit status](#exit-status)) both complete **before** the deadline.

`timeout_seconds` is configurable per registration or per invocation override at resolve time (I40). Engine default is **`provider_timeout_seconds_default`** (60 seconds; [cli-contract.md](cli-contract.md#resource-bounds-d008)).

## No state authority

Provider results are observations and recommendations only. No `result.kind`, payload field, or stderr content may:

- set or imply engine workflow state or lifecycle;
- append journal entries or evidence records directly;
- select a transition target or substitute gate identities;
- bypass engine graph resolution or persistence.

The engine alone commits state and journal facts after validating provider output.

## Five roles

Role names are stable protocol identifiers. They are **not** application operations (I40, D004).

| Role | Purpose |
|---|---|
| `describe` | Input-free emission of complete graph projection, optional input declarations, static guidance, and live-guidance capability. |
| `validate_inputs` | Value-only validation of candidate creation inputs. **Must not** return topology. |
| `evaluate_gates` | Evaluate all required gates for one transition in one invocation. |
| `live_guidance` | Explicitly requested advisory guidance for an active run. |
| `check_compatibility` | Non-latching capability findings against a stored graph. |

## Role-specific result applicability

Each role exposes only the `result.kind` values listed below. Roles **must not** invent generic `rejected` or other cross-role denial variants. Domain rejections are derived by the engine from role-valid results plus catalog rules (D004).

| Role | Permitted `result.kind` | Engine consumer mapping (summary) |
|---|---|---|
| `describe` | `description` only | Completed protocol result. Semantically invalid graph → `provider.graph.invalid` on creation or invalid conformance finding on `provider.check`. **No** role-valid denial kind. |
| `validate_inputs` | `accepted`, `rejected`, `evaluation_error` | `accepted` → creation continues. `rejected` → domain rejection `input.rejected`. `evaluation_error` → `provider.evaluation_error`. |
| `evaluate_gates` | `verdicts`, `incompatible`, `evaluation_error` | `verdicts` with all pass → transition allowed. Any fail verdict → `gate.failed` rejection. `incompatible` → `compatibility.unsupported` rejection. `evaluation_error` or malformed/missing verdict set → operation error. |
| `live_guidance` | `guidance`, `incompatible`, `evaluation_error` | `guidance` → completed advisory text. `incompatible` → `compatibility.unsupported` rejection. `evaluation_error` → `provider.evaluation_error`. No evidence append. |
| `check_compatibility` | `findings`, `evaluation_error` | `findings` (including incompatible capabilities) → **completed** operation. `evaluation_error` → `provider.evaluation_error`. Incompatibility inside findings is not a top-level rejection. |

### Transport and process failures (all roles)

These conditions are **operation errors** for every role. They never produce domain rejection:

| Condition | Reason code |
|---|---|
| Tombstoned registration | `provider.tombstoned` |
| Executable not found | `provider.executable.not_found` |
| Process-group establishment or PGID verification failure | `provider.spawn.failed` |
| Pre-spawn request size exceeds bound | `resource.exhausted` |
| Unsupported `protocol_major` | `provider.protocol.unsupported_major` |
| Malformed JSON, wrong tags, ID mismatch | `provider.protocol.malformed` |
| Oversized stdout result | `provider.protocol.oversized` |
| Invalid UTF-8 in request or stdout result JSON | `provider.protocol.invalid_utf8` |
| Timeout | `provider.timeout` |
| Crash / abort | `provider.crash` |
| Signal termination | `provider.signal` |
| Non-zero exit | `provider.nonzero_exit` |
| Role that permits `evaluation_error` returns it | `provider.evaluation_error` |
| Invalid graph after `description` on creation | `provider.graph.invalid` |
| Executable digest drift between paired creation calls | `provider.drift.detected` |
| Malformed provider evidence on gate verdict path | `provider.evidence.malformed` |

Full taxonomy: [operation-catalog.md](operation-catalog.md#outcome-and-reason-taxonomy).

### Catalog operation mapping

| Application operation | Invoked role(s) |
|---|---|
| `provider.check` (default) | `describe` |
| `provider.check` (`--active-runs`) | `describe`, then `check_compatibility` per active run row |
| `run.create` | `describe`; `validate_inputs` when declarations and/or candidate values exist |
| `run.request` (gated transition) | `evaluate_gates` |
| `run.request` (gate-free) | none |
| `run.guidance` | `live_guidance` |
| `run.compatibility` | `check_compatibility` |

Invoker integration performs catalog resolution **before** spawning; the provider executable never queries the catalog.

## Payload and result shapes (transport level)

Detailed wire schemas are implemented in T084 from [graph-projection.md](graph-projection.md). Transport requires these discriminant and containment rules:

### `describe`

- **Request `payload`:** `{}` — no candidate input values (I6, I32).
- **`result.kind`:** `description`
- **`result` fields:** `graph` (complete wire projection object per [graph-projection.md](graph-projection.md#provider-wire-graph-projection-protocol-v1)).
- **Graph identity:** integration maps `graph` to canonical bytes and computes `graph_revision` (`sha256:` + SHA-256 hex). Raw provider JSON formatting, key order, and array order **must not** define identity. `provider_version` is audit-only.

### `validate_inputs`

- **Request `payload`:** `declarations` (from prior describe), `candidate_values` (object).
- **`result.kind`:** `accepted` | `rejected` | `evaluation_error`
- **`accepted`:** optional normalized `values` object; **must not** include graph/topology keys.
- **`rejected`:** `diagnostics` array; maps to `input.rejected`.
- **`evaluation_error`:** `diagnostics` array.

### `evaluate_gates`

- **Request `payload`:** `snapshot` with bounded run context: graph identity, lifecycle, current state, requested event, required gate IDs, immutable inputs, inline evidence, caller-selected evidence references.
- **`result.kind`:** `verdicts` | `incompatible` | `evaluation_error`
- **`verdicts`:** `verdicts` array with exactly one entry per requested gate ID (`gate_id`, `passed` boolean), optional `evidence` array valid only on this kind.
- **`incompatible`:** `diagnostics` explaining stored-graph/capability mismatch.
- **`evaluation_error`:** `diagnostics` array.

### `live_guidance`

- **Request `payload`:** `snapshot` similar to gate context without transition authority.
- **`result.kind`:** `guidance` | `incompatible` | `evaluation_error`
- **`guidance`:** `text` string (at most `guidance_text_bytes`; [cli-contract.md](cli-contract.md#resource-bounds-d008)).
- **`incompatible`:** stored-guidance capability mismatch → `compatibility.unsupported` when engine selected live guidance.
- **`evaluation_error`:** `diagnostics` array. No evidence fields.

### `check_compatibility`

- **Request `payload`:** `stored_graph` object, optional `capabilities` filter list.
- **`result.kind`:** `findings` | `evaluation_error`
- **`findings`:** `capabilities` array of `{ "capability", "status", "diagnostics" }` where `status` is `compatible`, `incompatible`, or `unknown`. Incompatible entries are findings, not protocol-level rejection.
- **`evaluation_error`:** `diagnostics` array.

### Diagnostics object

Used in `rejected`, `incompatible`, and `evaluation_error` results:

```json
{
  "code": "string",
  "message": "string",
  "path": "optional JSON-pointer-like string"
}
```

## Examples

Examples use minimal placeholder graph objects. They are valid JSON and language-neutral.

### Example: `describe` request

```json
{
  "protocol_major": 1,
  "role": "describe",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b417d",
  "registration": {
    "registration_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
    "config_revision": 1,
    "executable": "/opt/providers/workflow",
    "argv": [],
    "working_directory": "/opt/providers",
    "timeout_seconds": 60
  },
  "payload": {}
}
```

### Example: describe result

```json
{
  "protocol_major": 1,
  "role": "describe",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b417d",
  "provider_version": "1.0.0",
  "result": {
    "kind": "description",
    "graph": {
      "initial_state": "draft",
      "states": [
        {
          "id": "draft",
          "static_guidance": "Prepare the change.",
          "final": false
        }
      ],
      "transitions": [],
      "input_declarations": [],
      "live_guidance_supported": false
    }
  }
}
```

### Example: `validate_inputs` rejected result

```json
{
  "protocol_major": 1,
  "role": "validate_inputs",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b417e",
  "result": {
    "kind": "rejected",
    "diagnostics": [
      {
        "code": "input.missing",
        "message": "Required input 'ticket' is missing.",
        "path": "/candidate_values/ticket"
      }
    ]
  }
}
```

### Example: `evaluate_gates` verdicts result

```json
{
  "protocol_major": 1,
  "role": "evaluate_gates",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b417f",
  "result": {
    "kind": "verdicts",
    "verdicts": [
      { "gate_id": "tests-passed", "passed": true },
      { "gate_id": "review-approved", "passed": false }
    ],
    "evidence": []
  }
}
```

### Example: `live_guidance` guidance result

```json
{
  "protocol_major": 1,
  "role": "live_guidance",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b4180",
  "result": {
    "kind": "guidance",
    "text": "Address review feedback on error handling before resubmitting."
  }
}
```

### Example: `check_compatibility` findings result

```json
{
  "protocol_major": 1,
  "role": "check_compatibility",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b4181",
  "result": {
    "kind": "findings",
    "capabilities": [
      {
        "capability": "gate:security-review",
        "status": "incompatible",
        "diagnostics": [
          {
            "code": "gate.removed",
            "message": "Stored graph requires gate 'security-review' which this build no longer implements."
          }
        ]
      },
      {
        "capability": "live_guidance",
        "status": "compatible",
        "diagnostics": []
      }
    ]
  }
}
```

### Example: `evaluation_error` (roles except `describe`)

```json
{
  "protocol_major": 1,
  "role": "evaluate_gates",
  "invocation_id": "019f6e88-b403-73a6-89f9-ebfe668b4182",
  "result": {
    "kind": "evaluation_error",
    "diagnostics": [
      {
        "code": "dependency.unavailable",
        "message": "Required external checker is not reachable."
      }
    ]
  }
}
```

## Provider author checklist

1. Read stdin until EOF; parse exactly one request envelope.
2. Do not treat unsupported `protocol_major` as role-specific `evaluation_error`; integration maps it to `provider.protocol.unsupported_major` for every role.
3. Branch on `role`; never return a `result.kind` invalid for that role.
4. Echo `invocation_id` and `role` in the result envelope.
5. Write exactly one JSON result to stdout within **`provider_result_stdout_bytes`**, then exit 0.
6. Use stderr only for human diagnostics (arbitrary bytes; not authoritative).
7. Optionally include `provider_version` in the result envelope for audit-only capture; integration preserves it for operational trace and durable journal/provider-observation facts when the invocation can be durably recorded (I8, I43); it never changes semantics.
8. Do not depend on catalog queries, persistent stdio sessions, or setting engine state.
9. Honor stored graph contracts or return explicit `incompatible` / incompatible findings / `evaluation_error` as applicable.

## Schema implementation boundary

T084 publishes `schemas/provider/v1/*.json` generated from this contract and [graph-projection.md](graph-projection.md). Canonical bytes and golden vectors are normative in [graph-projection.md](graph-projection.md#golden-vectors). Numeric bounds are named in [cli-contract.md](cli-contract.md#resource-bounds-d008) (D008, T008).
