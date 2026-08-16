---
name: using-loop-engine
description: Use when driving a durable Loop Engine workflow run from the CLI — confirming work-slot policy with the user before start, starting a run against a configured provider, inspecting or resuming a run, appending context records, requesting events, invoking bound work slots, terminating, interpreting completed/rejected/error outcomes, or fanning out worker CLIs without a run.
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

`--help` also lists `fan-out` beside, and distinct from, these eight. It is not a ninth primary operation.

Usual-case `start` omits `--database` and `artifact_root`:

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

`start` returns the run ID at `result.run.id`; supply `--id` when the orchestrator owns run identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged through result, `show`, and `history`. Append is opaque to core and never changes state.

## Lock work-slot policy before start

Bindings freeze with `initial_input` and cannot be patched. Do not call `start` until the user has actively approved the work-slot policy for this run. Copying a provider profile is not approval: some shipped profiles already contain `work_slot_bindings`.

Ask, show the exact JSON you will freeze, and wait for explicit confirmation of all three:

1. **Whether to bind any slots**, and if so which catalog slot IDs. Sparse map: a present key is a mandatory worker for that room; an absent key stays driver-performed. `{}` or omitting the key means no bindings.
2. **Command and args per bound slot** — keep that provider's shipped default argv, or replace it. Quote the exact `{command, args}` for every bound slot.
3. **Model identity per bound slot that will invoke a model-bearing CLI.** The engine freezes argv only. Encode the model in those frozen args (inner `--task-worker` JSON, repeated `--worker` JSON, or the worker CLI's own model flags). Nested inner workers count, not only the outer binding command. Do not choose a model after `start`.

Do not call `start` while any bound slot will invoke a model-bearing CLI (`pi --print`, `claude -p`, `run-plan-graph`'s inner worker, or similar) unless each model identifier is present in those frozen args, or the user has explicitly accepted that CLI's unpinned default as the model policy. Keeping a shipped outer argv that does not name a model is not model lock-in.

If the user declines bindings, delete `work_slot_bindings` or set it to `{}` in the run-specific input even when the shipped profile had defaults. A wrong freeze cannot be edited in place: terminate and start a new run.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: slot ID → `{command, args}`. Omit or `{}` means none. `start` rejects unknown slot IDs, unknown fields, and non-object values.

`show` projects `work_slots` (catalog: id, state, event) and `work_slot_invocations`. Overlay status is `running` | `succeeded` | `failed` | `overrun`. Compare `current_state` to frozen bindings to decide the path:

- **Unbound** (cataloged or not, but absent from frozen bindings): `current_state_instructions` is the stored work body. Perform that work yourself. Then append and request the event.
- **Bound** (slot ID present in frozen bindings): instructions name the slot, the frozen `{command, args}`, and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`. They omit the stored work body. Do not perform that body, and do not exec the frozen command yourself. `invoke`, then poll `show` until overlay `succeeded`, `failed`, or `overrun`. On `overrun`, `invoke` the same slot again. Overlay `succeeded` means the bound CLI exited 0, not that the change is accepted. You still append provider-shaped evidence and request the shown event.

`invoke RUN_ID SLOT_ID` starts the bound worker. Rejected for unknown, unbound, or overlay-`running` slots. Overlay `overrun` is terminal for retry — invoke the same slot again. The worker stdin JSON object is `run_id`, `slot_id`, `artifact_root`, and `instruction_body`. Hidden `wait-invocation` is parent of the worker; it is not a user command and not a daemon.

`event`/`evaluate` never wait on a worker. A bound checked edge requires overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database.

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE]
```

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Workers come only from repeated `--worker` JSON objects `{command, args}`. Zero `--worker` entries fail closed.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot; `invoke` execs the frozen argv with the existing worker packet on stdin. Bound mode rejects `--instructions`. Callers who want reviewers put `--worker` JSON objects in those frozen binding args at `start`. Bindings cannot be patched mid-run. Stock software-change review bindings freeze `design-review`, `plan-review`, and `implementation-review` to `loop-engine fan-out` with zero `--worker` entries; invoke of those slots therefore fails closed. Recovery is terminate and start again with `--worker` objects in the frozen args.

`software-change run-plan-graph` is an argv command of the software-change provider binary — the shipped implement worker — not an engine operation.

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
- Public-boundary proof uses `scripts/software-change-journey.py`; source full mode drives separate engine processes and packaged checked-prefix mode consumes only provider data materialized by `data-dump`. Policy-document and research journeys do the same for those providers. Each freezes a sparse dummy-worker `work_slot_bindings` entry and `invoke`s before the bound checked event. The software-change source full journey also proves `run-plan-graph` and `fan-out` with dummy inner workers (no live model), including zero-worker bound review invoke failing closed. Synthetic evidence proves deterministic mechanics, not semantic verdict quality.
- `initial_input` is immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Do not `start` until the user has approved the bindings JSON, including default-vs-custom argv and models encoded in those args — or an explicit unpinned-default acceptance for any model-bearing CLI that has no model in argv.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination — not every read, provider failure, overlay `overrun`, or other rejection; history is not provider context.
