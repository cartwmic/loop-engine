# Loop Engine Operational Trace

**Status:** Operational trace JSONL v1 schema, lifecycle, budgets, and failure behavior are settled.

This document is the canonical contract for per-invocation JSONL operational trace schema v1, file permissions, initialization and flush lifecycle, encoded-byte budget and cross-process rotation, provider payload retention without raw/parsed duplication, driver and pre-dispatch invocation behavior, late sink-failure truthfulness, and deterministic Unix `SIGXFSZ` / `RLIMIT_FSIZE` end-to-end proof. Trace directory location is defined in [configuration.md](configuration.md). Named numeric bounds are defined in [cli-contract.md](cli-contract.md#resource-bounds); this document references bound **names** only.

Related documents:

- [Resource bounds and trace budgets](cli-contract.md#resource-bounds)
- [Operational trace budgets (cross-process)](cli-contract.md#operational-trace-budgets-cross-process)
- [Dispatch boundary](cli-contract.md#dispatch-boundary)
- [Machine-local configuration](configuration.md)
- [Provider protocol v1](provider-protocol-v1.md)
- [Persistence contract](persistence.md)
- [Application operation catalog](operation-catalog.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [Testing doctrine](testing.md)
- [Interaction storyboards](ux-storyboards.md)
- [System invariants](invariants.md) — I18, I42, I46
- [Published schema index](../schemas/index.json)

## Scope and authority

Operational trace is **diagnostic storage only** (I42, I46). It is **not**:

- authoritative workflow state or lifecycle;
- the activity journal or evidence store;
- a second write path for mutations;
- a replay or reconstruction source for current state.

Rotation may remove closed trace files. Abrupt process death, storage failure, rotation, and late sink failure can limit completeness; the engine **MUST NOT** claim impossible complete observation (I46).

Instrumentation stays at three stable choke points ([architecture.md](architecture.md) § Operational diagnostics):

| Choke point | Trace obligation |
|---|---|
| CLI operation dispatcher | Request ID, operation identity, bounded request payload, completed/rejected/error outcome |
| Provider subprocess integration | Locator, digest/version, configured arguments, bounded protocol request/result, captured stdout/stderr, timing, execution outcome |
| Persistence integration | Read or write scope intent, applicable workflow/lifecycle version check when present, and commit/rollback outcome for writes or read_complete/read_failure outcome for reads |

Core emits **targeted** `decision.*` events only for consequential transition, gate, lifecycle, compatibility, and stale-state outcomes not fully explained by boundary payloads. Pure helpers carry no logging requirement. Production code **MUST NOT** gain alternate provider, persistence, or dispatch paths that bypass these choke points.

Rejected patterns:

- threading trace context through every function;
- mandatory per-function logging attributes;
- custom compiler plugins for log counting;
- treating trace as competing authority over SQLite or export files.

## Trace inspection

Every invocation returns or prints a trace path except when trace initialization itself fails. Treat each file as JSON Lines: parse one line at a time, verify every `request_id` matches the filename and CLI envelope, then follow `invocation.start` → optional boundary events → `invocation.outcome` → `invocation.finish`. Provider stdout/stderr appears only inside bounded `provider.finish`/`provider.failure` detail; it is never terminal output.

For diagnosis:

```bash
trace=/path/from/the/cli/envelope.jsonl
while IFS= read -r line; do printf '%s\n' "$line" | python3 -m json.tool; done < "$trace"
```

Missing terminal lines do not prove rollback: inspect authoritative state with `run.show`, `run.history`, `run.list`, or `provider.list`. A `trace.sink_failure` after commit is diagnostic only. Closed traces may be rotated, so copy a relevant closed file before reproducing high-volume failures. Do not change trace/reservation files or permissions while an invocation is active.

## File layout and permissions

| Artifact | Path | Mode |
|---|---|---|
| Trace directory | `{machine_home_root}/traces/` per [configuration.md](configuration.md) | `0700` |
| Per-invocation trace file | `{machine_home_root}/traces/{request_id}.jsonl` | `0600` |
| Rotation coordinator lock | `{machine_home_root}/traces/.rotation.lock` | `0600` (created atomically) |
| Per-invocation reservation sidecar | `{machine_home_root}/traces/.reserve/{request_id}.json` | `0600` (created atomically) |

On supported Unix platforms, the engine **MUST** create missing parent directories with mode `0700` and each new trace file with mode `0600`. Permission failure before dispatch is a trace-initialization failure: no application operation dispatch, no provider subprocess, and no persistence mutation.

`request_id` is an opaque stable identifier (at most `identifier_utf8_bytes`) shared by the trace filename, CLI `request_id` / `trace` fields, and every JSONL line's `request_id` field.

Inherited process environment is **never** copied into trace (I42).

## Initialization, write, and flush lifecycle

Every CLI invocation — including `--help`, `--version`, `--list-operations`, argv/flag parse failures, and configuration load failures that occur after trace init — follows this sequence when trace initialization succeeds:

1. **Allocate** `request_id` and resolve trace directory.
2. **Coordinate** cross-process rotation (see [Cross-process budget and rotation](#cross-process-budget-and-rotation)): acquire `.rotation.lock`, reconcile crash-stale reservation sidecars, evict eligible closed files, create `.reserve/{request_id}.json` with `unused_reservation_bytes` = `trace_init_reservation_bytes`, and update directory accounting atomically.
3. **Create** `{request_id}.jsonl` with mode `0600`; hold an exclusive lock on the reservation sidecar for the invocation lifetime.
4. **Write** `invocation.start` as the first line under `.rotation.lock` (append, flush, `fsync`, and decrement `unused_reservation_bytes` by the encoded line delta before releasing the lock).
5. **Dispatch** driver metadata or application operation; for each subsequent line, hold `.rotation.lock` across append, flush, and atomic `unused_reservation_bytes` decrement; add provider-call unused reservation before spawn and release unused remainder on provider-call close, both under `.rotation.lock`.
6. **Write** terminal `invocation.finish` when the invocation reaches a defined terminal boundary; **flush** and `fsync` under `.rotation.lock` with the same decrement rule.
7. **Close** the trace file, reacquire `.rotation.lock`, remove the reservation sidecar (actual `{request_id}.jsonl` size remains in directory accounting), release the sidecar lock, and release `.rotation.lock`.

Trace initialization failure occurs when steps 1–4 cannot complete (insufficient directory budget after eviction, permission failure, unwritable media). Such failure **blocks dispatch** and surfaces rich stderr per [cli-contract.md](cli-contract.md#dispatch-boundary).

Post-initialization trace sink failure **MUST NOT** roll back a committed persistence mutation. The public outcome **MUST** report the true operation result when authoritative verification proves commit ([persistence.md](persistence.md) § Commit outcome verification). When observable, attach a `trace.sink_failure` diagnostic to the outcome envelope or stderr pre-dispatch object; **MUST NOT** misreport `completed` as `error` or claim rollback for committed state.

## JSONL v1 line envelope

Each line is one UTF-8 JSON object terminated by `\n`. Readers **MUST** parse linewise. Within major version `1`, same-major evolution is additive: new optional fields may appear; readers ignore unknown fields. Embedded protocol and graph payloads follow the strict parse rules in [provider-protocol-v1.md](provider-protocol-v1.md) and [graph-projection.md](graph-projection.md): duplicate object keys and trailing values after the first complete JSON value are rejected.

### Common fields (every line)

| Field | Type | Required | Description |
|---|---|---|---|
| `trace_schema_version` | integer | yes | Always `1` for this contract |
| `ts` | string | yes | RFC 3339 UTC timestamp with millisecond precision |
| `request_id` | string | yes | Invocation identifier; matches filename stem |
| `category` | string | yes | Event category (see below) |
| `event` | string | yes | Event name within category |

### Categories and events

| `category` | `event` | When emitted |
|---|---|---|
| `invocation` | `start` | First line after successful file creation |
| `invocation` | `request` | Application operation dispatched with bounded request DTO |
| `invocation` | `outcome` | Full bounded CLI outcome envelope embedded once (no duplicated top-level fields) |
| `invocation` | `finish` | Terminal line with `exit_code` |
| `invocation` | `error` | Pre-dispatch dispatcher failure before `finish` when no `outcome` was emitted (for example `trace_init`, `platform`, `config`, or `persistence` open/migration/schema/integrity) |
| `driver` | `metadata` | Successful `--help`, `--version`, or `--list-operations` |
| `parse` | `failure` | Pre-dispatch parse/usage failure after trace init |
| `provider` | `start` | Immediately before provider subprocess spawn |
| `provider` | `finish` | Provider returned; bounded payloads attached |
| `provider` | `failure` | Provider process/protocol failure |
| `persistence` | `intent` | Persistence read or write scope opened with stated mutation class |
| `persistence` | `version_check` | Workflow/lifecycle version read for CAS |
| `persistence` | `commit` | Write transaction successful `COMMIT` |
| `persistence` | `rollback` | Write transaction rolled back without durable effect |
| `persistence` | `read_complete` | Read-only or export snapshot read completed |
| `persistence` | `read_failure` | Read-only or export snapshot read failed |
| `decision` | `transition` | State transition resolved (accepted or denied) |
| `decision` | `gate` | Gate verdict set or gate denial context |
| `decision` | `lifecycle` | Lifecycle denial or terminal transition |
| `decision` | `compatibility` | Compatibility finding recorded |
| `decision` | `stale` | Stale version or registration recheck denial |
| `trace` | `sink_failure` | Post-init trace write failure |
| `trace` | `rotation_evict` | Closed trace file removed by coordinator |

### Category payloads

**`invocation.start`**

Common fields on every `invocation.start` line:

| Field | Type | Required | Description |
|---|---|---|---|
| `format` | string | yes | `human` or `json` |
| `platform` | string | yes | Detected target triple |
| `argv_byte_length` | integer | yes | Total UTF-8 bytes of process argv before any trace capture truncation |

**Dispatched application operations** (those that emit `invocation.request`) **MUST** record argv digest metadata only and **MUST NOT** duplicate the parsed request DTO:

| Field | Type | Required | Description |
|---|---|---|---|
| `argv_digest` | string | yes | SHA-256 hex of UTF-8 bytes formed by joining raw argv elements with U+0000 separators |

**Driver metadata, parse/usage diagnostics, and other pre-dispatch invocations** that do not emit `invocation.request` **MUST** retain a bounded raw argv prefix for diagnostics:

| Field | Type | Required | Description |
|---|---|---|---|
| `argv` | array of strings | yes | Bounded prefix of raw argv elements; each element truncated to at most `filesystem_path_utf8_bytes` UTF-8 bytes |
| `argv_truncated` | boolean | yes | `true` when any argv element or total captured argv exceeded capture bounds |

Argv capture is a **size bound**, not secret redaction. Trace does not claim to remove or mask sensitive argv content beyond what bounds omit.

**`invocation.request`**

| Field | Type | Required |
|---|---|---|
| `operation` | string | yes | Application operation ID from [operation-catalog.md](operation-catalog.md) |
| `request` | object | yes | Bounded operation request DTO embedded once as JSON value |

**`invocation.outcome`**

| Field | Type | Required | Description |
|---|---|---|---|
| `envelope` | object | yes | Full bounded CLI outcome envelope embedded once as JSON value (not a JSON string) per [cli-contract.md](cli-contract.md#structured-outcome-envelope-v1); **MUST** include all required top-level fields (`schema_version`, `operation`, `request_id`, `trace`, `outcome`, `reason`, `data`, `diagnostics`) exactly once with `request_id` and `trace` mutually consistent with the trace line |

**`invocation.finish`**

| Field | Type | Required |
|---|---|---|
| `exit_code` | integer | yes | Process exit code (`0`, `2`, `1`, or `64`) |

**`driver.metadata`**

| Field | Type | Required |
|---|---|---|
| `kind` | string | yes | `help`, `version`, or `list_operations` |

**`parse.failure`**

| Field | Type | Required |
|---|---|---|
| `phase` | string | yes | `parse` or `usage` |
| `message` | string | yes | Human-readable summary |
| `source_chain` | array of strings | no | Nested context, root cause last |

**`invocation.error`**

Emitted for pre-dispatch dispatcher failures after trace initialization when no `invocation.outcome` is written. Parse/usage failures after trace init use `parse.failure` instead; persistence open/migration/schema/integrity failures use `invocation.error` with `phase` `persistence`.

| Field | Type | Required |
|---|---|---|
| `phase` | string | yes | `trace_init`, `platform`, `config`, or `persistence` |
| `message` | string | yes | Human-readable summary |
| `source_chain` | array of strings | no | Nested context, root cause last |

**`provider.start`**

| Field | Type | Required |
|---|---|---|
| `invocation_id` | string | yes | Provider protocol correlation ID |
| `role` | string | yes | Provider role name |
| `registration_id` | string | yes | Resolved registration ID |
| `executable` | string | yes | Absolute executable path used |
| `argv` | array of strings | yes | Configured `registration.argv` |
| `working_directory` | string | yes | Configured working directory |
| `timeout_seconds` | integer | yes | Wall-clock timeout |

**`provider.finish` / `provider.failure`** (shared shape; `failure` adds `failure_code`)

| Field | Type | Required |
|---|---|---|
| `invocation_id` | string | yes | Provider protocol correlation ID |
| `role` | string | yes | Provider role name |
| `request` | object | yes | Protocol request envelope embedded **once** as JSON value per [provider-protocol-v1.md](provider-protocol-v1.md#request-envelope); **MUST** include all required fields (`protocol_major`, `role`, `invocation_id`, `registration`, `payload`) exactly once |
| `stdout_b64` | string | yes | Retained stdout prefix, base64 (empty string when none); exact when in bound |
| `stdout_original_length` | integer | yes | Original stdout byte length before truncation |
| `stdout_truncated` | boolean | yes | `true` when `stdout_original_length` > `provider_result_stdout_bytes` |
| `stderr_b64` | string | yes | Retained stderr prefix, base64 |
| `stderr_byte_length` | integer | yes | Original stderr byte length before truncation |
| `stderr_truncated` | boolean | yes | `true` when `stderr_byte_length` > `provider_stderr_trace_bytes` |
| `result_digest` | string or null | yes | SHA-256 hex of parsed stdout JSON when parse succeeded; `null` otherwise |
| `result_byte_length` | integer or null | yes | Parsed result document byte length when available |
| `duration_ms` | integer | yes | Wall-clock duration |
| `exit_status` | integer or null | yes | Raw wait status when available |
| `failure_code` | string | no | Present on `failure` only (for example `provider.timeout`) |

Parsed provider result objects are **not** duplicated. Only `result_digest` and `result_byte_length` metadata appear when stdout parses.

**`persistence.intent`**

| Field | Type | Required |
|---|---|---|
| `mutation_class` | string | yes | `catalog`, `run_create`, `run_mutation`, `read_only`, or `export_read` |
| `operation` | string | yes | Application operation ID |

**`persistence.version_check`**

| Field | Type | Required |
|---|---|---|
| `run_id` | string | no | Present for run-scoped CAS |
| `expected_workflow_version` | integer | no | |
| `expected_lifecycle_version` | integer | no | |
| `registration_config_revision` | integer | no | Catalog CAS |

**`persistence.commit` / `persistence.rollback`** (write paths only)

| Field | Type | Required |
|---|---|---|
| `operation` | string | yes | Application operation ID |
| `outcome` | string | yes | `completed`, `rejected`, or `error` semantic class persisted |

**`persistence.read_complete`** (read paths only)

| Field | Type | Required | Description |
|---|---|---|---|
| `operation` | string | yes | Application operation ID |
| `outcome` | string | yes | `completed` or `rejected` semantic class surfaced to the operation layer; not mutation authority |
| `item_count` | integer | no | Paged `read_only` operations: items returned in this page |
| `next_cursor_present` | boolean | no | Paged `read_only` operations: whether `data.next_cursor` is present |
| `page_data_byte_length` | integer | no | Encoded `data.items` bytes for the returned page (bounded by `collection_page_data_budget_bytes`) |
| `result_digest` | string or null | no | SHA-256 hex of canonical encoded read `data` subset when computable; `null` when empty |
| `manifest_digest` | string | no | `export_read` only: SHA-256 hex of on-disk `manifest.json` bytes |
| `artifact_byte_lengths` | object | no | `export_read` only: map of relative artifact path to byte length from published manifest inventory |

**`persistence.read_failure`** (read paths only)

| Field | Type | Required | Description |
|---|---|---|---|
| `operation` | string | yes | Application operation ID |
| `failure_code` | string | yes | Stable reason code (for example `persistence.failed`, `persistence.busy`) |
| `message` | string | no | Human-readable summary |

**`decision.*`**

| Field | Type | Required |
|---|---|---|
| `operation` | string | yes | Application operation ID |
| `run_id` | string | no | When run-scoped |
| `detail` | object | yes | Bounded decision facts (transition IDs, gate IDs, capability keys, reason codes) |

**`trace.sink_failure`**

| Field | Type | Required |
|---|---|---|
| `errno` | string | yes | Platform error name (for example `EFBIG`, `ENOSPC`) |
| `phase` | string | yes | `write`, `flush`, or `fsync` |
| `after_commit` | boolean | yes | `true` when durable mutation already verified |

**`trace.rotation_evict`**

| Field | Type | Required |
|---|---|---|
| `evicted_path` | string | yes | Absolute path removed |
| `encoded_bytes_reclaimed` | integer | yes | Bytes freed |

## Provider payload encoding (no duplication)

Per [cli-contract.md](cli-contract.md#operational-trace-budgets-cross-process):

- Provider **request** is embedded once as a JSON object value in `provider.finish` / `provider.failure`.
- Provider **stdout** is stored once as base64 in `stdout_b64` with `stdout_original_length` and `stdout_truncated`; when `stdout_original_length` ≤ `provider_result_stdout_bytes`, retention is exact; when larger, retain a prefix up to `provider_result_stdout_bytes` and drain the remainder without storing it.
- Provider **stderr** is stored once as base64 in `stderr_b64` with `stderr_byte_length` and `stderr_truncated`; remainder is drained but not stored ([provider-protocol-v1.md](provider-protocol-v1.md) § Stderr).
- **Parsed** stdout JSON is never stored again; only `result_digest` and `result_byte_length`.
- Dispatcher **request** is embedded once in `invocation.request`; dispatcher **outcome** is embedded once in `invocation.outcome` as the full envelope object only (no duplicated top-level `outcome`/`reason` fields), never as escaped JSON strings.

Budget counting uses **on-disk encoded line bytes** including JSON framing, escaping, and base64 expansion.

## Persistence read outcome (no mutation authority)

Read paths use `mutation_class` `read_only` or `export_read`. Every opened persistence read **MUST** emit `persistence.intent` then exactly one `persistence.read_complete` or `persistence.read_failure` before `invocation.outcome`, including provider-free reads and `run.export` snapshot reads.

- Read outcomes record diagnostic persistence-boundary closure only. They **MUST NOT** use `persistence.commit` or `persistence.rollback` and do not confer mutation authority over SQLite, journal rows, or export artifacts.
- `persistence.commit` and `persistence.rollback` apply only to write mutation classes (`catalog`, `run_create`, `run_mutation`).
- Dispatcher read results are embedded once in `invocation.outcome`; trace retains bounded metadata only (`item_count`, `next_cursor_present`, `page_data_byte_length`, `result_digest`, `manifest_digest`, `artifact_byte_lengths`).
- Paged `read_only` operations **MAY** include `item_count`, `next_cursor_present`, and `page_data_byte_length` (encoded `data.items` bytes for the returned page).
- `export_read` **MUST** include `manifest_digest` on `read_complete` and **MAY** include `artifact_byte_lengths` mapping relative artifact paths to byte lengths from the published manifest inventory per [export-contract.md](export-contract.md).

## Cross-process budget and rotation

Numeric bounds are defined only in [cli-contract.md](cli-contract.md#resource-bounds). This section states behavior.

| Phase | Rule |
|---|---|
| Trace initialization | After evicting eligible closed files, create sidecar with `unused_reservation_bytes` = `trace_init_reservation_bytes` against `trace_directory_budget_bytes` |
| Before each provider call | Under `.rotation.lock`, add `trace_provider_call_reservation_bytes` to `unused_reservation_bytes` before spawn |
| After each line write | Under `.rotation.lock`, append encoded line and decrement `unused_reservation_bytes` by the line byte delta; JSONL growth and decrement are atomic — bytes are never double-counted |
| Provider call close | Under `.rotation.lock`, release unused remainder of that call from `unused_reservation_bytes` |
| Rotation coordinator | Holds `.rotation.lock`; directory usage = sum of `{request_id}.jsonl` actual sizes plus sum of live sidecar `unused_reservation_bytes` values, compared to `trace_directory_budget_bytes` and `trace_retained_files_max` |

Eligible eviction targets are **closed** trace files only. An **open** trace file is never removed or truncated by rotation. A trace is **open** while its reservation sidecar exists and is held under an exclusive lock by the owning process.

### Cross-process reservation sidecar protocol (Unix)

Cross-process budget coordination uses Rust `std` file locking only. No leases, heartbeats, or competing authority.

| Artifact | Role |
|---|---|
| `.rotation.lock` | Exclusive lock serializes every coordinator mutation: stale-sidecar reconciliation, eviction, reservation create/update/remove, trace writes with sidecar decrement, and directory accounting |
| `.reserve/{request_id}.json` | Mode `0600` sidecar recording `unused_reservation_bytes` (unused reservation remainder for the invocation); held under an exclusive lock by the owning process for the invocation lifetime |
| `{request_id}.jsonl` | Source of truth for **actual encoded bytes** on disk |

Each sidecar contains two fixed-size generation slots. Every slot complements both generation and reservation values. Updates overwrite only older slot, flush, and fsync; readers select highest complete valid generation. Torn newest slot therefore falls back to prior value: reservation additions fail before dispatch, while trace writes and releases can only leave conservative over-accounting.

**Directory accounting invariant** — while holding `.rotation.lock`, directory usage **MUST** equal the sum of all `{request_id}.jsonl` actual encoded byte sizes plus the sum of `unused_reservation_bytes` from every live `.reserve/{request_id}.json` sidecar. Each trace write and sidecar decrement **MUST** occur under the same `.rotation.lock` hold so no observer sees an intermediate double count.

**Normal path**

1. Acquire exclusive lock on `.rotation.lock`.
2. Reconcile any crash-stale sidecars (see below).
3. Evict eligible closed traces; recompute directory usage from JSONL sizes and live sidecars.
4. Create `.reserve/{request_id}.json` with initial `unused_reservation_bytes` = `trace_init_reservation_bytes` and acquire an exclusive lock on the sidecar before releasing `.rotation.lock`.
5. On each encoded line write, hold `.rotation.lock` across append, flush, and atomic `unused_reservation_bytes` decrement by the encoded byte delta. Before each provider subprocess spawn, reacquire `.rotation.lock` and add `trace_provider_call_reservation_bytes` to `unused_reservation_bytes`. On provider-call close, reacquire `.rotation.lock` and release the call's unused remainder from `unused_reservation_bytes`, then release.
6. On invocation close, reacquire `.rotation.lock`, remove the sidecar (actual `{request_id}.jsonl` bytes remain counted via directory scan), release locks.

**Crash-stale sidecar reconciliation**

When a process dies without closing its trace, the kernel releases the sidecar lock. The next coordinator that acquires `.rotation.lock` **MUST**, for each `.reserve/{request_id}.json` sidecar whose exclusive lock is acquirable immediately (no owning process):

1. Reconcile through the normal directory scan: actual `{request_id}.jsonl` bytes are already included in the JSONL size sum; measure the file when present (treat missing file as `0`) only to confirm closure state.
2. Remove the sidecar, subtracting its `unused_reservation_bytes` from directory accounting. **MUST NOT** add measured actual bytes again.
3. Treat the trace as **closed** and eligible for eviction.

Other processes **MUST** count a live sidecar's `unused_reservation_bytes` toward `trace_directory_budget_bytes` even when they cannot observe the owner's in-process remainder directly.

**Per-file cap** — worst-case reservation: `trace_init_reservation_bytes` + `provider_calls_per_paged_invocation_max` × `trace_provider_call_reservation_bytes` = 16 MiB + 10 × 10 MiB = 116 MiB, below `trace_file_max_bytes` (120 MiB).

**Per-call cap** — worst-case encoded provider event stays below `trace_provider_call_reservation_bytes` (10 MiB) per [cli-contract.md](cli-contract.md#operational-trace-budgets-cross-process).

**Failure mapping**

| Condition | Behavior |
|---|---|
| Insufficient base reservation at initialization | Trace-initialization failure; pre-dispatch exit `64` |
| Insufficient next-call reservation after page progress | End page with `next_cursor` |
| Insufficient reservation before any row returned | `resource.exhausted` with unchanged cursor |
| Active file would exceed `trace_file_max_bytes` | Treat as post-init `trace.sink_failure`; do not truncate committed persistence outcome |

Concurrent CLI processes **MUST** serialize rotation decisions through `.rotation.lock` with bounded retry; lock contention is not a user-facing error unless initialization cannot proceed.

## Driver, help, version, and parse behavior

Per I46 and [cli-contract.md](cli-contract.md#dispatch-boundary):

| Invocation kind | Trace required | Typical events | stdout | Exit |
|---|---|---|---|---:|
| `--help`, `--version`, `--list-operations` | yes | `start` → `driver.metadata` → `finish` | driver output | `0` |
| Pre-dispatch parse/usage after trace init | yes | `start` → `parse.failure` → `finish` | empty | `64` |
| Pre-dispatch config/platform/trace_init failure | only if trace allocated | `start` (optional) → `invocation.error` → `finish` (optional) or no file | empty | `64` |
| Pre-dispatch persistence open/migration/schema/integrity failure | yes | `start` → `invocation.error` (`phase`: `persistence`) → `finish` | empty | `64` |
| Application operation | yes | `start` → `request` → (`provider.*` / `persistence.*` / `decision.*`)\* → `outcome` → `finish` | outcome envelope | `0`/`2`/`1` |

Driver metadata invocations **MUST NOT** emit `invocation.request` or `invocation.outcome` with an application `operation` field. Structured stderr for pre-dispatch failures follows [cli-contract.md](cli-contract.md#pre-dispatch-failure-object-stderr-structured-mode); trace file **MAY** also contain matching `parse.failure` or `invocation.error` when trace initialization succeeded. Post-dispatch persistence operation errors (`reason.code` `persistence.failed`, exit `1`) emit `invocation.outcome` and **MUST NOT** use pre-dispatch `invocation.error`.

## Late sink failure truthfulness

| Scenario | Persistence truth | Public outcome | Trace |
|---|---|---|---|
| Sink fails before any mutation attempt | unchanged | true `rejected`/`error`/`completed` for read-only paths | `trace.sink_failure` when observable |
| Sink fails after successful `COMMIT` | committed | **true** `completed`/`rejected`/`error` per verification; **MUST NOT** claim rollback | `trace.sink_failure` with `after_commit: true`; `invocation.finish` **MAY** be absent |
| Commit I/O unknown | verify on fresh connection | per [persistence.md](persistence.md) § Commit outcome verification | partial trace acceptable |
| Init failure | no mutation | rich stderr; no stdout envelope | no file or partial `start` only |

Diagnostics from late sink failure appear in outcome `diagnostics` when envelope construction succeeds; they **MUST NOT** change `outcome` class for verified commits.

## Deterministic Unix `SIGXFSZ` / `RLIMIT_FSIZE` E2E contract

Production code **MUST NOT** contain test branches for file-size limits. E2Es use an **external wrapper** that:

1. Ignores `SIGXFSZ` in the wrapper process.
2. Sets `RLIMIT_FSIZE` to a deterministic byte ceiling for the child `loop-engine` process.
3. Invokes the production binary with isolated `LOOP_ENGINE_HOME`.

Cases run only on supported macOS and Linux hosts ([testing.md](testing.md) § Supported platform scope).

### Case A — provider-dependent late `EFBIG`

**Goal:** prove late sink failure after init does not falsify a truthful read/report outcome when persistence is unaffected.

**Setup:** choose `RLIMIT_FSIZE` above trace initialization and `invocation.start` but below the encoded size of the next `provider.start` event (for example `provider.check` default conformance).

**Assert:**

- exit and envelope reflect true operation result (`completed` or `error` per provider outcome);
- committed catalog/run state unchanged when operation is read-only;
- trace contains flushed `invocation.start` and, when reached, `trace.sink_failure` with `errno` `EFBIG`;
- `invocation.finish` may be absent;
- envelope **MUST NOT** claim rollback or mutation that did not occur.

### Case B — provider-free committed annotation

**Goal:** prove durable mutation survives late trace sink failure and fresh-process read.

**Setup:** `run.annotate` (or equivalent provider-free journal append) with `RLIMIT_FSIZE` above initialization and `invocation.start` / `persistence.intent` but below `persistence.commit` and `invocation.outcome` events.

**Assert:**

- annotation/history visible via fresh-process `run.history` or `run.show`;
- structured envelope reports `outcome: completed` (not `error` claiming rollback);
- trace contains `persistence.commit` when flushed before limit, or authoritative DB proves commit when trace truncated;
- `trace.sink_failure` with `after_commit: true` when observable;
- no production code branch keyed on test environment.

## Operation event closure (21-operation target catalog)

All 21 operations reported by `--list-operations` are dispatched and enforce the rows below. Every dispatched application operation **MUST** emit at minimum: `invocation.start`, `invocation.request`, `invocation.outcome`, and `invocation.finish` unless crash or late sink failure prevents terminal lines.

| Operation ID | `persistence.*` | `provider.*` | `decision.*` |
|---|---|---|---|
| `provider.add` | intent, commit or rollback | — | — |
| `provider.list` | read_only intent, read_complete or read_failure | — | — |
| `provider.check` | read_only intent, read_complete or read_failure | start/finish or failure per call | compatibility when `--active-runs` |
| `provider.update` | intent, version_check, commit or rollback | — | — |
| `provider.rename` | intent, commit or rollback | — | — |
| `provider.disable` | intent, commit or rollback (mutating path only) | — | — |
| `provider.restore` | intent, commit or rollback | — | — |
| `run.create` | intent, commit or rollback | describe (+ validate_inputs when inputs present) | transition on success |
| `run.list` | read_only intent, read_complete or read_failure | — | — |
| `run.show` | read_only intent, read_complete or read_failure | — | — |
| `run.graph` | read_only intent, read_complete or read_failure | — | — |
| `run.history` | read_only intent, read_complete or read_failure | — | — |
| `run.evidence.add` | intent, commit or rollback | — | — |
| `run.evidence.list` | read_only intent, read_complete or read_failure | — | — |
| `run.annotate` | intent, commit or rollback | — | — |
| `run.label` | intent, commit or rollback | — | — |
| `run.request` | intent, version_check, commit or rollback | when gates require provider | transition, gate, lifecycle, stale as applicable |
| `run.guidance` | intent, commit or rollback | start/finish or failure | lifecycle, compatibility as applicable |
| `run.compatibility` | intent, commit or rollback | start/finish or failure | compatibility |
| `run.terminate` | intent, commit or rollback | — | lifecycle |
| `run.export` | export_read intent, read_complete or read_failure | — | — |

Provider-free operations still emit `persistence.intent` with `mutation_class` `read_only` or `export_read` as applicable, followed by `persistence.read_complete` or `persistence.read_failure`. Rejected post-lookup mutations emit `persistence.rollback` or `persistence.commit` with semantic `rejected` per [persistence.md](persistence.md). Read paths **MUST NOT** emit `commit` or `rollback`.

## Contract examples

Paths and IDs are illustrative. Each block below is one JSONL line unless noted.

### Completed — `run.show`

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"invocation","event":"start","argv_digest":"53b5397c9c9eb2bd17679d1bfcd87ec2eb9d1a0556df22b591b7a7bc5447f314","argv_byte_length":46,"format":"json","platform":"aarch64-apple-darwin"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.001Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"invocation","event":"request","operation":"run.show","request":{"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X"}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.002Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"persistence","event":"intent","mutation_class":"read_only","operation":"run.show"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.025Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"persistence","event":"read_complete","operation":"run.show","outcome":"completed"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.050Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"invocation","event":"outcome","envelope":{"schema_version":1,"operation":"run.show","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","trace":"/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V1W.jsonl","outcome":"completed","reason":null,"data":{"run":{"id":"01J9X3K2M4N5P6Q7R8S9T0V2X","lifecycle":"active","state":"explore","state_changed":false},"requestable_events":["intent-ready"]},"diagnostics":[]}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:00:00.051Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V1W","category":"invocation","event":"finish","exit_code":0}
```

### Completed — `run.export` (export snapshot read)

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"invocation","event":"start","argv_digest":"c2f5d0e8b1a94e3f7d6c8a2b5e9f1d4c6a8b0e2f4d6c8a1b3e5f7d9c2a4b6e8","argv_byte_length":58,"format":"json","platform":"aarch64-apple-darwin"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.001Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"invocation","event":"request","operation":"run.export","request":{"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X","output":"/tmp/export-target"}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.002Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"persistence","event":"intent","mutation_class":"export_read","operation":"run.export"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.120Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"persistence","event":"read_complete","operation":"run.export","outcome":"completed","manifest_digest":"8f14e45fceea167a5a36dedd4bea2543cf1c2ad5c4c6b8b8b8b8b8b8b8b8b8b8","artifact_byte_lengths":{"journal.jsonl":4096,"manifest.json":512,"state.json":8192}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.150Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"invocation","event":"outcome","envelope":{"schema_version":1,"operation":"run.export","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","trace":"/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V8D.jsonl","outcome":"completed","reason":null,"data":{"export":{"output":"/tmp/export-target","manifest_file":"manifest.json","state_file":"state.json","journal_file":"journal.jsonl"}},"diagnostics":[]}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:05:00.151Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V8D","category":"invocation","event":"finish","exit_code":0}
```

### Rejected — `run.request` (gate failed)

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"invocation","event":"start","argv_digest":"9b340e8176583d1679907fbbdc297276abdb641945679174677a2ab138e7af3e","argv_byte_length":54,"format":"human","platform":"x86_64-unknown-linux-gnu"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.010Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"invocation","event":"request","operation":"run.request","request":{"run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X","event":"ship"}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.100Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"decision","event":"gate","operation":"run.request","run_id":"01J9X3K2M4N5P6Q7R8S9T0V2X","detail":{"event":"ship","verdict":"failed","gate_id":"release-review"}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.150Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"persistence","event":"commit","operation":"run.request","outcome":"rejected"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.200Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"invocation","event":"outcome","envelope":{"schema_version":1,"operation":"run.request","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","trace":"/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V3Y.jsonl","outcome":"rejected","reason":{"code":"gate.failed","message":"Gate release-review failed"},"data":{"run":{"id":"01J9X3K2M4N5P6Q7R8S9T0V2X","label":"checkout-redesign","lifecycle":"active","state":"design-review","state_changed":false},"evidence_recorded":{"inline":true,"selected_associations":true,"provider":true},"requestable_events":["approved","changes-requested"]},"diagnostics":[]}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:01:00.201Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V3Y","category":"invocation","event":"finish","exit_code":2}
```

### Error — `run.create` (provider protocol)

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:02:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V4Z","category":"provider","event":"failure","invocation_id":"pv-001","role":"describe","request":{"protocol_major":1,"role":"describe","invocation_id":"pv-001","registration":{"registration_id":"019f6e88-b403-73a6-89f9-ebfe668b417e","config_revision":1,"executable":"/opt/providers/workflow","argv":[],"working_directory":"/opt/providers","timeout_seconds":60},"payload":{}},"stdout_b64":"","stdout_original_length":0,"stdout_truncated":false,"stderr_b64":"YmFkIGdyYXBo","stderr_byte_length":9,"stderr_truncated":false,"result_digest":null,"result_byte_length":null,"duration_ms":120,"exit_status":1,"failure_code":"provider.protocol.malformed"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:02:00.050Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V4Z","category":"persistence","event":"rollback","operation":"run.create","outcome":"error"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:02:00.100Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V4Z","category":"invocation","event":"outcome","envelope":{"schema_version":1,"operation":"run.create","request_id":"01J9X3K2M4N5P6Q7R8S9T0V4Z","trace":"/Users/example/.local/state/loop-engine/traces/01J9X3K2M4N5P6Q7R8S9T0V4Z.jsonl","outcome":"error","reason":{"code":"provider.protocol.malformed","message":"Provider returned malformed describe result"},"data":{},"diagnostics":[{"code":"provider.invocation","message":"Role describe returned malformed protocol result","context":{"role":"describe","failure_code":"provider.protocol.malformed"}}]}}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:02:00.101Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V4Z","category":"invocation","event":"finish","exit_code":1}
```

### Parse failure — pre-dispatch (stderr + trace)

Trace file:

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:03:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V5A","category":"invocation","event":"start","argv":["loop-engine","run","list","--limt","10"],"argv_byte_length":30,"argv_truncated":false,"format":"json","platform":"aarch64-apple-darwin"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:03:00.001Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V5A","category":"parse","event":"failure","phase":"parse","message":"unknown flag --limt","source_chain":["run list: unrecognized flag --limt"]}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:03:00.002Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V5A","category":"invocation","event":"finish","exit_code":64}
```

Matching stderr (structured mode) per [cli-contract.md](cli-contract.md): `phase` `parse`, exit `64`, no stdout envelope.

### Trace initialization failure

No trace file (or empty). Stderr only (human or pre-dispatch JSON with `phase` `trace_init`). **No** `invocation.request`, **no** provider events, **no** persistence mutation, exit `64`.

### Persistence open failure — pre-dispatch (stderr + trace)

Trace file:

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:04:00.000Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V7C","category":"invocation","event":"start","argv":["loop-engine","run","list"],"argv_byte_length":18,"argv_truncated":false,"format":"json","platform":"aarch64-apple-darwin"}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:04:00.001Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V7C","category":"invocation","event":"error","phase":"persistence","message":"Database migration failed","source_chain":["migration 0002_add_indexes: UNIQUE constraint failed"]}
```

```json
{"trace_schema_version":1,"ts":"2026-07-17T10:04:00.002Z","request_id":"01J9X3K2M4N5P6Q7R8S9T0V7C","category":"invocation","event":"finish","exit_code":64}
```

Matching stderr (structured mode) per [cli-contract.md](cli-contract.md#pre-dispatch--persistence-open-failure-stderr-structured-mode): `phase` `persistence`, exit `64`, no stdout envelope. **No** `invocation.request`, **no** provider events, **no** persistence mutation.

### Crash — blocking provider (partial trace)

Last flushed line may be `provider.start` or an incomplete read path after `persistence.intent` without `read_complete`/`read_failure`. **No** `invocation.finish` required. Pre-effect markers must identify last observed phase. No durable mutation when no `persistence.commit` occurred.

## Published trace event schema

The common JSONL line envelope is indexed at [schemas/index.json](../schemas/index.json) as `schemas/trace/v1/event.schema.json` (`trace_schema_version` `1`, title `TraceEvent`). Category-specific fields remain normative in this document; the schema captures required common fields and bound markers only.

| Property | Value |
|---|---|
| Exposure | **Published** — generated from integration types |
| Generation | `cargo run -p loop-engine-integrations --example generate_trace_schema` (writes JSON Schema to stdout; published artifact path above) |
| Validation | `cargo test -p loop-engine-integrations published_trace_fixtures_are_versioned_and_never_duplicate_parsed_stdout` |
| Fixtures | `schemas/trace/v1/fixtures/*.json` (non-normative line-shape examples) |

Trace is diagnostic storage only. It is not authoritative state, does not replay into current workflow state, and never substitutes for SQLite or export artifacts.

## Verification rules

- JSONL contract examples parse linewise as valid JSON.
- Numeric behavior references bound names in [cli-contract.md](cli-contract.md#resource-bounds) only; per-file reservation sum (`trace_init_reservation_bytes` + `provider_calls_per_paged_invocation_max` × `trace_provider_call_reservation_bytes` = 116 MiB) stays below `trace_file_max_bytes` (120 MiB).
- Operation event closure table covers all 21 application operation IDs from [operation-catalog.md](operation-catalog.md); every `read_only` and `export_read` row closes with `read_complete` or `read_failure`, never `commit`/`rollback`.
- Trace persistence boundary facet ([testing.md](testing.md) § Facet matrix, [quality/facets/v1/README.md](../quality/facets/v1/README.md)) closes on attempted read/transaction, applicable version check, and commit/rollback/read outcome per operation exposure trace.
- Every opened persistence read emits `persistence.intent` then exactly one `persistence.read_complete` or `persistence.read_failure`; read outcomes are not mutation authority.
- Late sink failure semantics never falsify verified committed outcomes.
- `SIGXFSZ` / `RLIMIT_FSIZE` cases A and B are specified for external-wrapper E2Es without production test branches.
- Trace is diagnostic only; no document assigns trace competing authority over SQLite or journal.
- Directory mode `0700`, file mode `0600`, line flush/`fsync` on init and finish documented.
- Provider payloads stored once; no parsed stdout duplication; oversized stdout retains bounded prefix with `stdout_original_length` and `stdout_truncated` (in-bound stdout exact).
- `invocation.outcome` embeds the full CLI envelope once with all required [cli-contract](cli-contract.md#structured-outcome-envelope-v1) top-level fields (`schema_version`, `operation`, `request_id`, `trace`, `outcome`, `reason`, `data`, `diagnostics`); top-level `outcome`/`reason` fields are not duplicated outside `envelope`.
- `provider.finish` / `provider.failure` embed the full provider protocol request once with all required [provider-protocol-v1](provider-protocol-v1.md#request-envelope) fields (`protocol_major`, `role`, `invocation_id`, `registration`, `payload`); parsed stdout is not duplicated.
- `invocation.start` uses `argv_digest` metadata for dispatched operations and bounded raw `argv` only for driver/parse diagnostics; capture bounds are not secret redaction.
- Cross-process rotation uses `.rotation.lock` plus per-invocation `.reserve/{request_id}.json` sidecars with field `unused_reservation_bytes`; under `.rotation.lock`, directory usage equals sum of JSONL actual sizes plus live sidecar unused values; trace writes and sidecar decrements are atomic under the same lock; crash-stale unlocked sidecars are removed after directory-scan reconciliation without double-counting actual bytes; open traces never evicted.
- Help, version, list-operations, and parse failures initialize trace per I46 when initialization succeeds.

## Deliberate exclusions

- Trace as write authority or journal substitute.
- Per-function mandatory logging or trace context threading.
- Impossible crash-complete trace guarantees.
- Production `RLIMIT_FSIZE` / fault-injection branches.
- Windows trace permission semantics are deferred.
