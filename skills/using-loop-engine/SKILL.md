---
name: using-loop-engine
description: Use when driving a durable Loop Engine workflow run from the CLI — starting a run against a configured provider, inspecting or resuming a run, appending context records, requesting events, terminating, or interpreting completed/rejected/error outcomes.
---

# Using Loop Engine

## Overview

`loop-engine` owns durable run state, the stored workflow graph, and the progression decision. You perform the primary work externally; the engine never edits repositories, documents, or artifacts. Workflows come from external provider executables configured in TOML.

Full semantics: [docs/agent-usage.md](../../docs/agent-usage.md). Requirements: [docs/PRD.md](../../docs/PRD.md).

## Deterministic setup

- Pass an explicit `--database PATH` on every invocation and reuse it for the run's lifetime.
- Pass `--json` on every invocation; parse only the single JSON envelope.
- Pass an explicit `--config PATH` (provider TOML) to `start`; do not rely on discovery.
- `--timeout-ms N` is global (default 30000 per provider `describe`/`evaluate` call); raise it for slow providers.

Provider TOML uses an exact, case-sensitive alias:

```toml
[providers.software-change]
command = "/absolute/path/to/provider"
args = []
```

`start` initial input and `append` data accept JSON inline, `@FILE`, or `-` (stdin).

## Commands

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
```

`start` returns the run ID at `result.run.id`; supply `--id`/`--record-id` when the orchestrator owns identity. `append` is opaque to core and never changes state.

## Canonical loop

Repeat until `show` reports `final` or `terminated`:

1. `show` — read `current_state` (a state ID) with its sibling fields `current_state_title` and `current_state_instructions`, immutable `initial_input`, ordered `context`, `requestable_events`, and `latest_evaluations` (includes latest checked-transition denial feedback).
2. Perform the instructed work externally.
3. `append` durable context for evidence, findings, decisions, or steering. Core assigns no meaning to `kind`/`data`; follow provider/state conventions. Checked evaluations receive `initial_input` plus all context in stable append order.
4. Request exactly one event from `requestable_events`. Append any final handoff context before an event entering a final state.
5. Inspect the envelope, then `show` again before the next event. On `rejected`, follow the feedback and continue work; on `error`, assume nothing advanced and re-read `show`.

Request events, never states. Only events listed by the latest `show` are available from the current state.

## Outcomes

| Envelope | Meaning | Exit |
|---|---|---|
| `status: "completed"` + `operation` + `result` | Operation succeeded | 0 |
| `status: "rejected"` + `operation`/`code`/`message` + optional `details` | Understood, denied — checked-transition denials carry durable actionable feedback | 10 |
| `status: "error"` + `operation`/`code`/`message` + optional `details` | Could not be reliably evaluated or committed; provider failure/`unsupported` lands here | 20 |
| `status: "invalid-invocation"` (only `status`/`code`/`message`) | Malformed CLI syntax or input | 2 |

Parse JSON even on nonzero exit. Treat only `completed` as success. Never infer state advancement from `rejected` or `error` — re-run `show` against the same database.

## Rules

- One logical mutating actor per run: serialize `append`, `event`, and `terminate` calls; never race them from parallel workers. Concurrent reads are fine. Context appended during an in-flight checked evaluation does not invalidate or reach that evaluation.
- `initial_input` is immutable run configuration; never attempt to replace it.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, and termination — not every read, provider failure, or other rejection; history is not provider context.
