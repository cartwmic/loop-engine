---
name: using-loop-engine
description: Use when driving a durable Loop Engine workflow run from the CLI — confirming work-slot policy with the user before start, previewing bindings, starting a run against a configured provider, inspecting or resuming a run, appending context records, requesting events, invoking bound work slots, terminating, interpreting completed/rejected/error/invalid-invocation outcomes, or fanning out worker CLIs without a run.
---

# Using Loop Engine

## Overview

`loop-engine` owns durable run state, the stored workflow graph, and the progression decision. You perform the primary work externally; the engine never edits repositories, documents, or artifacts. Workflows come from external provider executables configured in TOML. Provider `describe` and `evaluate` are deterministic and do not invoke a model.

This skill is the engine source of truth. Reference provider skills embed a driving minimum and name this skill as a required companion; they do not replace it.

Full semantics: [docs/agent-usage.md](../../docs/agent-usage.md). Requirements: [docs/PRD.md](../../docs/PRD.md).

## Deterministic setup

When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority.

- When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file.
- Pass `--json` and parse the single JSON envelope.
- Pass an explicit `--config PATH` (provider TOML) to `start`; do not rely on discovery.
- That start stores the allocated absolute path in object `initial_input`. `list` JSON includes optional `provider` (the start alias) and `artifact_root`; `show` and `history` JSON keys are unchanged.
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

`--help` also lists `fan-out` and `preview-bindings` under Other commands, beside and distinct from these eight. They are not a ninth primary operation.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

`start` returns the run ID at `result.run.id`; supply `--id` when the orchestrator owns run identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged through result, `show`, and `history`. Append is opaque to core and never changes state.

## Lock work-slot policy before start

Bindings freeze with `initial_input` and cannot be patched. Do not call `start` until the user has actively approved the work-slot policy for this run. Copying a provider profile is not approval. Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`). Bound slots are opt-in: copy a skill template into the per-run profile JSON after replacing placeholders. Inspect the JSON with `preview-bindings` before that confirmation.

Ask, show the exact JSON you will freeze, and wait for explicit confirmation of all three:

1. **Whether to bind any slots**, and if so which catalog slot IDs. Sparse map: a present key is a mandatory worker for that room; an absent key stays driver-performed. `{}` or omitting the key means no bindings.
2. **Command and args per bound slot** — copy a skill template (fill `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL` in the per-run JSON, not in the skill file), or write custom argv. Quote the exact `{command, args}` for every bound slot.
3. **Model identity per bound slot that will invoke a model-bearing CLI.** The engine freezes argv only. Encode the model in those frozen args (inner `--task-worker` JSON, repeated `--worker` JSON, or the worker CLI's own model flags). Nested inner workers count, not only the outer binding command. Do not choose a model after `start`.

Do not call `start` while any bound slot will invoke a model-bearing CLI (`pi --print`, `claude -p`, `run-plan-graph`'s inner worker, or similar) unless each model identifier is present in those frozen args, or the user has explicitly accepted that CLI's unpinned default as the model policy. An outer argv that does not name a model is not model lock-in.

If the user declines bindings, omit `work_slot_bindings` or set it to `{}`. A wrong freeze cannot be edited in place: terminate and start a new run.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: slot ID → `{command, args}`. Omit or `{}` means none. `start` rejects unknown slot IDs, unknown fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv.

`show` projects `work_slots` (catalog: id, state, event) and `work_slot_invocations`. Overlay status is `running` | `succeeded` | `failed` | `overrun`. Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). `show` does not spawn a provider and does not read capture files. Overlay meaning:

- succeeded: the bound CLI exited 0, not that the provider accepted the work
- failed: the bound CLI exited nonzero or the waiter vanished
- running: the waiter is alive and allowed time has not elapsed
- overrun: allowed time elapsed while the waiter is alive; invoke the same slot again

Compare `current_state` to frozen bindings to decide the path:

- **Unbound** (cataloged or not, but absent from frozen bindings): `current_state_instructions` is the stored work body. Perform that work yourself. Then append and request the event.
- **Bound** (slot ID present in frozen bindings): instructions name the slot, the frozen `{command, args}`, and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`. They omit the stored work body. Do not perform that body, and do not exec the frozen command yourself. `invoke`, then poll `show` until overlay `succeeded`, `failed`, or `overrun`. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun invoke the same slot again; on failed inspect stderr.

`invoke RUN_ID SLOT_ID` starts the bound worker. Rejected for unknown, unbound, or overlay-`running` slots. Overlay `overrun` is terminal for retry — invoke the same slot again. On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. The worker stdin JSON object is exactly `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`. Hidden `wait-invocation` is parent of the worker; it is not a user command and not a daemon. After waitpid, if `capture_dir/summary.json` is well-formed, the waiter copies inner `command`/`args`/`exit_code` onto the invocation; overlay remains the bound CLI process exit (0 → succeeded).

`event`/`evaluate` never wait on a worker. A bound checked edge requires overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database.

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE]
```

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Workers come only from repeated `--worker` JSON objects `{command, args}`. Zero `--worker` entries fail closed.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot; `invoke` execs the frozen argv with the existing worker packet on stdin. Bound mode honors `packet.capture_dir` (writes per-worker `0/`, `1/`, … plus `summary.json` there) and rejects `--instructions`. Callers who want reviewers put `--worker` JSON objects in those frozen binding args at `start` after `preview-bindings` and lock-in. Bindings cannot be patched mid-run.

Shipped software-change profiles omit `work_slot_bindings` (or `{}`), so `implement`, `design-review`, `plan-review`, and `implementation-review` stay driver-performed. Bound workers are opt-in. A usable review binding is caller-supplied `--worker` objects frozen at `start` after preview and lock-in — not a stock zero-worker `fan-out` argv.

Copy-paste templates into the **per-run** profile JSON after replacing `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL`. Do not put machine-local paths in the skill file. Pi examples keep `--no-skills --no-extensions` and add explicit `-e` so cursor-provider and claude-bridge load. Review `pi` workers include `--tools read,grep,find,ls` and must not pass `--no-context-files`. Implement examples do not add `--tools`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning.

Opt-in review example (every model-bearing worker names a model):

```json
"design-review": {
  "command": "loop-engine",
  "args": [
    "fan-out",
    "--worker", "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--tools\",\"read,grep,find,ls\",\"--model\",\"MODEL\"]}"
  ]
}
```

Opt-in implement example:

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--model\",\"MODEL\"]}"
  ]
}
```

`software-change run-plan-graph` is an argv command of the software-change provider binary — not an engine operation. Bound mode honors `packet.capture_dir` (per-task `{task_id}/` plus `summary.json`). When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths.

## Non-run-state command: preview-bindings

`preview-bindings` does not start, advance, or record a run and does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key.

It expands nested `--worker` and `--task-worker` JSON `{command, args}` objects, lists detected `--model` values, and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. Warnings alone exit 0. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

Repeat until `show` reports `final` or `terminated`:

1. `show` — read `current_state` (a state ID) with its sibling fields `current_state_title` and `current_state_instructions`, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations` (heartbeat fields above), ordered `context`, `requestable_events`, and `latest_evaluations` (includes latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` it and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`; on overlay `overrun`, `invoke` again. Follow the bound-instruction triage order. Otherwise perform the instructed work externally.
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
- Public-boundary proof uses `scripts/software-change-journey.py`; source full mode drives separate engine processes and packaged checked-prefix mode consumes only provider data materialized by `data-dump`. Policy-document and research journeys do the same for those providers. Each freezes a sparse dummy-worker `work_slot_bindings` entry and `invoke`s before the bound checked event. The software-change source full journey also proves `run-plan-graph` and `fan-out` with dummy inner workers (no live model), including `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run. Synthetic evidence proves deterministic mechanics, not semantic verdict quality.
- `initial_input` is immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Do not `start` until the user has approved the bindings JSON, including default-vs-custom argv and models encoded in those args — or an explicit unpinned-default acceptance for any model-bearing CLI that has no model in argv.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination — not every read, provider failure, overlay `overrun`, or other rejection; history is not provider context.
