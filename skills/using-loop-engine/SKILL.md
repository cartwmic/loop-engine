---
name: using-loop-engine
description: Use when driving a durable Loop Engine workflow run from the CLI — starting a run against a configured provider, inspecting or resuming a run, appending context records, requesting events, invoking bound work slots, terminating, or interpreting completed/rejected/error outcomes.
---

# Using Loop Engine

## Overview

`loop-engine` owns durable run state, the stored workflow graph, and the progression decision. You perform the primary work externally; the engine never edits repositories, documents, or artifacts. Workflows come from external provider executables configured in TOML.

Full semantics: [docs/agent-usage.md](../../docs/agent-usage.md). Requirements: [docs/PRD.md](../../docs/PRD.md).

## Deterministic setup

- Omit `--database` unless isolating. When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file. Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`.
- Pass `--json` and parse the single JSON envelope.
- Pass an explicit `--config PATH` (provider TOML) to `start`; do not rely on discovery.
- Omit `artifact_root` unless isolating files. Usual start without `artifact_root` stores the allocated absolute path in object `initial_input`. Pass a nonempty `artifact_root` only to isolate files to a caller-chosen absolute existing directory. `list` JSON includes optional `provider` (the start alias) and `artifact_root`; `show` and `history` JSON keys are unchanged.
- `--timeout-ms N` is global (default 30000 per provider `describe`/`evaluate` call, and for `invoke` as `allowed_time_ms`); raise it for slow providers or long bound workers.

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
loop-engine [--database DB] [--json] [--timeout-ms MS] invoke RUN_ID SLOT_ID
```

Usual-case `start` omits `--database` and `artifact_root`:

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

`start` returns the run ID at `result.run.id`; supply `--id` when the orchestrator owns run identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged through result, `show`, and `history`. Append is opaque to core and never changes state.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: slot ID → `{command, args}`. Omit or `{}` means none. `start` rejects unknown slot IDs, unknown fields, and non-object values.

`show` projects `work_slots` and `work_slot_invocations`. Overlay status is `running` | `succeeded` | `failed` | `overrun`. When the current state is bound, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body. Do not perform that body yourself.

`invoke RUN_ID SLOT_ID` starts the bound worker. Rejected for unknown, unbound, or overlay-`running` slots. Overlay `overrun` is terminal for retry — invoke the same slot again. The worker stdin JSON object is `run_id`, `slot_id`, `artifact_root`, and `instruction_body`. Hidden `wait-invocation` is parent of the worker; it is not a user command and not a daemon.

`event`/`evaluate` never wait on a worker. A bound checked edge requires overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject.

## Canonical loop

Repeat until `show` reports `final` or `terminated`:

1. `show` — read `current_state` (a state ID) with its sibling fields `current_state_title` and `current_state_instructions`, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations`, ordered `context`, `requestable_events`, and `latest_evaluations` (includes latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` it and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`; on overlay `overrun`, `invoke` again. Otherwise perform the instructed work externally.
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

- One logical mutating actor per run: serialize `append`, `event`, `invoke`, and `terminate` calls; never race them from parallel workers. Concurrent reads are fine. Context appended during an in-flight checked evaluation does not invalidate or reach that evaluation.
- Production-boundary proof uses `scripts/production-journey.py`; source full mode drives separate engine processes and packaged checked-prefix mode consumes only provider data materialized by `data-dump`. Synthetic evidence proves deterministic mechanics, not semantic verdict quality.
- `initial_input` is immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination — not every read, provider failure, overlay `overrun`, or other rejection; history is not provider context.
