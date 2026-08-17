# Agent usage

Use `loop-engine` to coordinate durable workflow state. Perform the primary work externally; the engine owns the run state, stored workflow graph, and progression decision. Read [PRD.md](PRD.md) for full semantics.

## Deterministic setup

Omit `--database` unless isolating. When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file. Pass `--database /path/to/dir/loop.db` only to isolate SQLite and `/path/to/dir/runs/<id>/`.

Pass `--json` and parse the single JSON envelope. Pass an explicit `--config` path to `start`; do not rely on path discovery.

Omit `artifact_root` unless isolating files. Usual start without `artifact_root` stores the allocated absolute path in object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. Pass a nonempty `artifact_root` only to isolate files to a caller-chosen absolute existing directory. `list` JSON includes optional `provider` (the start alias) and `artifact_root`; `show` and `history` JSON keys are unchanged.

`--timeout-ms MILLISECONDS` is global and defaults to 30000 ms for each provider `describe` or `evaluate` call, and for `invoke` as the invocation's `allowed_time_ms`. Use an explicit timeout when provider latency or bound-worker runtime may exceed that default.

Provider TOML uses an exact, case-sensitive alias and a command invocation:

```toml
[providers.software-change]
command = "/absolute/path/to/provider"
args = []
```

`command` must be executable (or resolvable by the process environment). Keep `args` in the required order. The `start` initial input and `append` data accept JSON inline, from `@FILE`, or from `-` (stdin). These are JSON input sources only for those two operations; quote inline JSON for the shell.

## Command skeleton

Use the canonical executable name and these eight primary forms:

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MILLISECONDS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invoke RUN_ID SLOT_ID
```

`--help` also lists `fan-out` and `preview-bindings` under Other commands, beside and distinct from these eight. They are not a ninth primary operation.

Usual-case `start` omits `--database` and `artifact_root`:

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

Global options may appear before or after the operation; the timeout option is shown once above to keep the skeleton brief. `start` resolves the provider alias and returns the run ID in `result.run.id`; supply `--id` when the orchestrator owns run identity. Supply `--record-id` for an orchestrator-owned context-record identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; it accepts opaque `KIND` and `DATA_JSON` and does not change state.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: a sparse map from catalog slot ID to `{command, args}`. Omit the key or pass `{}` for no bindings. `start` rejects unknown slot IDs, unknown binding fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv. Bindings freeze with the run.

Lock that map with the user **before** `start`. Copying a provider profile is not approval. Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`). Bound slots are opt-in: copy a skill template into the per-run profile JSON after replacing `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL`. Inspect the JSON with `preview-bindings` before that confirmation. Confirm whether any slots are bound, the exact `{command, args}` for each bound slot, and — when a bound CLI will invoke a model — which model identifiers are encoded in those frozen args. Nested inner workers (`--task-worker`, `--worker`) count. Do not call `start` while a bound model-bearing CLI has no model in argv unless the user has explicitly accepted that CLI's unpinned default. Sparse: present keys are mandatory workers; absent keys stay driver-performed. A wrong freeze cannot be patched; terminate and start a new run.

`show` projects `work_slots` (catalog snapshot: id, state, event; no instruction body) and `work_slot_invocations` (including overlay status `running` | `succeeded` | `failed` | `overrun`). Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). `show` does not spawn a provider and does not read capture files. Overlay meaning: succeeded means the bound CLI exited 0, not that the provider accepted the work; failed means the bound CLI exited nonzero or the waiter vanished; running means the waiter is alive and allowed time has not elapsed; overrun means allowed time elapsed while the waiter is alive and the driver invokes the same slot again. When the current state is a bound slot, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun invoke the same slot again; on failed inspect stderr. Unbound slots keep the stored instruction body and the driver-performed path.

Start bound work with `invoke RUN_ID SLOT_ID`. It is rejected for an unknown slot, an unbound slot, or an overlay-`running` invocation. Overlay `overrun`, `failed`, and `succeeded` are not already-running. Overlay `overrun` is terminal for retry: invoke the same slot again. `event` and provider `evaluate` never wait on a worker.

On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`. The packet is not on argv, environment, or a temp file. Hidden `wait-invocation` is parent of that worker, waitpids it, and writes terminal `succeeded`/`failed`; after waitpid, a well-formed `capture_dir/summary.json` is copied as `inner_workers` (`command`, `args`, `exit_code`); overlay remains the bound CLI process exit (0 → succeeded). It is not a user command and not a daemon. A vanished waiter with no terminal status is overlay-`failed`.

A bound checked edge advances only after overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject. Overlay `running`/`failed`/`overrun` do not satisfy. Check-free edges are ungated.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database.

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE]
```

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Workers come only from repeated `--worker` JSON objects `{command, args}`. Zero `--worker` entries fail closed.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot; `invoke` execs the frozen argv with the existing worker packet on stdin. Bound mode honors `packet.capture_dir` (writes per-worker `0/`, `1/`, … plus `summary.json` there) and rejects `--instructions`. Callers who want reviewers put `--worker` JSON objects in those frozen binding args at `start` after `preview-bindings` and lock-in. Bindings cannot be patched mid-run.

Shipped software-change profiles omit `work_slot_bindings` (or `{}`), so `implement`, `design-review`, `plan-review`, and `implementation-review` stay driver-performed. Bound workers are opt-in. A usable review binding is caller-supplied `--worker` objects frozen at `start` after preview and lock-in — not a stock zero-worker `fan-out` argv.

Copy-paste templates into the **per-run** profile JSON after replacing `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL`. Do not put machine-local paths in skill files. Pi examples keep `--no-skills --no-extensions` and add explicit `-e` so cursor-provider and claude-bridge load. Review `pi` workers include `--tools read,grep,find,ls` and must not pass `--no-context-files`. Implement examples do not add `--tools`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning.

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

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key. It expands nested `--worker` and `--task-worker` JSON `{command, args}` objects, lists detected `--model` values, and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. Warnings alone exit 0. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

For an active run, repeat this exact handoff loop:

1. Run `show`; inspect `current_state`, its title and instructions, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations` (heartbeat fields above), ordered `context`, `requestable_events`, and `latest_evaluations` (including latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` that slot and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`; do not perform the stored work body. On overlay `overrun`, `invoke` the same slot again. Follow the bound-instruction triage order. Otherwise perform the instructed work externally. Do not expect the engine to edit the repository, document, or other external work.
3. Append durable context for useful evidence, findings, decisions, or steering. Context is opaque to Loop Engine core, not necessarily to the provider or workflow; follow provider/state conventions for `kind` and `data`. Core assigns no truth, provenance, approval, or supersession meaning. Every checked evaluation receives immutable `initial_input` and all accumulated context in stable append order.
4. Select one event from `requestable_events` and request exactly that `event`. Append any final handoff context before requesting an event that enters a final state.
5. Inspect the JSON status, then run `show` again before selecting another event. On `rejected`, follow its feedback and continue work; on `error`, do not assume anything advanced and re-read `show` (use `history` when auditing).
6. Stop when `show` reports `final` or `terminated`; these runs are read-only: `append`, `event`, and `terminate` are rejected without semantic history, and no requestable progression is exposed.

Request events, never target states. The engine resolves the target from the run's stored workflow. Request only events returned by the latest `show`; an event valid elsewhere in the graph is unavailable from the current state.

## JSON results

With `--json`, an operation returns one envelope. A successful operation includes `operation`, `status: "completed"`, and `result`; for example, `{"operation":"list","status":"completed","result":[]}`. An understood-but-denied request has `status: "rejected"` plus `code`, `message`, and optional `details`. An operation that cannot be reliably evaluated or committed has `status: "error"` with the same issue fields. Malformed CLI syntax or input returns only `status`, `code`, and `message`—no `operation`, `result`, or `details`:

```json
{"status":"invalid-invocation","code":"invalid-invocation","message":"missing required event ID"}
```

Exit codes map to `0` completed, `10` rejected, `20` error, and `2` invalid invocation. Parse the JSON even on a nonzero exit. Treat only `completed` as operation success. Never infer state advancement from `rejected` or `error`; re-run `show` against the same database. A checked-transition provider denial is `rejected` and exposes durable actionable feedback. Provider failure or `unsupported` is `error` and does not authorize progression. `history` includes checked-transition denials and work-slot invocation started/status-changed actions, not every read, provider failure, overlay `overrun`, or diagnostic; other rejections, such as unavailable events or terminal mutations, are absent.

## Handoff and provider boundary

`show` is a provider-free durable handoff: it reads the database and does not spawn or query the provider. A fresh agent needs the run ID, access to the same database, and access to every external reference carried in the initial input, context, or state instructions. It does not need the previous conversation or raw history to resume normal work. History is not context and is not supplied wholesale to provider evaluation; use it for semantic audit.

At `start`, the provider alias resolves once to its configured `command` and ordered `args`; that association, plus the provider's workflow topology and state-instruction snapshot, is stored with the run. Changing TOML cannot redirect an existing run. Executable contents at the stored command path may change; a later provider implementation evaluates the stored workflow snapshot and may return `unsupported` if it no longer supports it. Each provider `describe` or checked-transition `evaluate` call starts a fresh subprocess, sends one JSON request on stdin, and reads one JSON response on stdout; stderr is diagnostic only. A provider describes the workflow and evaluates the exact transition selected by the engine. It cannot choose a different target or set current state. Check-free transitions do not invoke it.

## Agent rules

- One logical mutating actor per run: serialize `append`, `event`, `invoke`, and `terminate` calls; never race them from parallel workers. Concurrent reads are fine. Context appended during an in-flight checked evaluation does not invalidate or reach that evaluation.
- Treat `initial_input` as immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Confirm that map with the user before `start`, including models in frozen args or an explicit unpinned-default acceptance.
- Treat context `kind` and `data` as opaque to Loop Engine core, but follow provider/state conventions for them; records are immutable and append-only.
- Use unique stable `--id` and `--record-id` values whenever an orchestrator controls identity. Append accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged in append results, `show` context, and durable `history`.
- Request only an event shown for the current state, and request one at a time.
- Follow rejection feedback; append corrected evidence or steering before retrying the shown event.
- Do not progress final or terminated runs; `show` exposes no requestable events there.
- Use `history` for a semantic audit of creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination; other rejections (such as unavailable events or terminal mutations) are absent, overlay `overrun` is not a history action, and history is not an exhaustive execution trace.

## Executable proof boundaries

Repository checks name boundaries they actually cross:

- component tests cover parser, core, SQLite, provider schema/evidence, and protocol behavior;
- composed tests combine engine operations, SQLite, and provider subprocesses below CLI process boundary;
- `scripts/software-change-journey.py --mode source --traversal-depth full` drives separate `loop-engine` processes through provider TOML, SQLite, production `software-change` process, copied high-rigor artifacts, sparse dummy-worker `work_slot_bindings`, `invoke` before the bound checked event, deterministic denials, evidence aggregation, and terminal state. It also proves unbound shipped profiles, `software-change run-plan-graph` and `loop-engine fan-out` with dummy inner workers (no live model), `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, and `preview-bindings` warning when pi has `--no-extensions` and no `-e`;
- `scripts/software-change-journey.py --mode packaged --traversal-depth checked-prefix` consumes extracted binaries, calls `data-dump`, and runs release-critical checked transition from dumped data only, including the same sparse binding/`invoke` proof;
- `scripts/policy-document-journey.py` drives draft and audit modes through provider TOML, SQLite, production `policy-document` process, a sparse dummy-worker binding on `semantic-review`, ungated `prepare` → `ready`, and deterministic/semantic denials;
- `scripts/research-journey.py --mode source` drives separate `loop-engine` processes through provider TOML, SQLite, production `research` process, copied standard profile, sparse dummy-worker `work_slot_bindings`, `invoke` before `scoped`, schema/evidence denials, and terminal state;
- `scripts/research-journey.py --mode packaged` consumes extracted binaries, calls `data-dump`, and runs the release-critical checked prefix from dumped data only, including the same sparse binding/`invoke` proof.

Journey evidence records are synthetic, schema-conforming pass records. They prove deterministic policy mechanics, author independence, revision-link handling, routing, aggregation, and persistence; they do not prove semantic review or verdict quality. Semantic review remains external.
