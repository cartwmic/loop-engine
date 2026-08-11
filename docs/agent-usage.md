# Agent usage

Use `loop-engine` to coordinate durable workflow state. Perform the primary work externally; the engine owns the run state, stored workflow graph, and progression decision. Read [PRD.md](PRD.md) for full semantics.

## Deterministic setup

Pass an explicit `--database` path on every invocation and reuse it for the run's lifetime. Pass `--json` on every invocation. Pass an explicit `--config` path to `start`; do not rely on path discovery or environment defaults.

`--timeout-ms MILLISECONDS` is global and defaults to 30000 ms for each provider `describe` or `evaluate` call. Use an explicit timeout when provider latency may exceed that default.

Provider TOML uses an exact, case-sensitive alias and a command invocation:

```toml
[providers.software-change]
command = "/absolute/path/to/provider"
args = []
```

`command` must be executable (or resolvable by the process environment). Keep `args` in the required order. The `start` initial input and `append` data accept JSON inline, from `@FILE`, or from `-` (stdin). These are JSON input sources only for those two operations; quote inline JSON for the shell.

## Command skeleton

Use the canonical executable name and these seven primary forms:

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MILLISECONDS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
```

Global options may appear before or after the operation; the timeout option is shown once above to keep the skeleton brief. `start` resolves the provider alias and returns the run ID in `result.run.id`; supply `--id` when the orchestrator owns run identity. Supply `--record-id` for an orchestrator-owned context-record identity. `append` accepts opaque `KIND` and `DATA_JSON` and does not change state.

## Canonical loop

For an active run, repeat this exact handoff loop:

1. Run `show`; inspect `current_state`, its title and instructions, immutable `initial_input`, ordered `context`, `requestable_events`, and `latest_evaluations` (including latest checked-transition denial feedback).
2. Perform the instructed work externally. Do not expect the engine to edit the repository, document, or other external work.
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

Exit codes map to `0` completed, `10` rejected, `20` error, and `2` invalid invocation. Parse the JSON even on a nonzero exit. Treat only `completed` as operation success. Never infer state advancement from `rejected` or `error`; re-run `show` against the same database. A checked-transition provider denial is `rejected` and exposes durable actionable feedback. Provider failure or `unsupported` is `error` and does not authorize progression. `history` includes checked-transition denials, not every read, invocation, provider failure, or diagnostic; other rejections, such as unavailable events or terminal mutations, are absent.

## Handoff and provider boundary

`show` is a provider-free durable handoff: it reads the database and does not spawn or query the provider. A fresh agent needs the run ID, access to the same database, and access to every external reference carried in the initial input, context, or state instructions. It does not need the previous conversation or raw history to resume normal work. History is not context and is not supplied wholesale to provider evaluation; use it for semantic audit.

At `start`, the provider alias resolves once to its configured `command` and ordered `args`; that association, plus the provider's workflow topology and state-instruction snapshot, is stored with the run. Changing TOML cannot redirect an existing run. Executable contents at the stored command path may change; a later provider implementation evaluates the stored workflow snapshot and may return `unsupported` if it no longer supports it. Each provider `describe` or checked-transition `evaluate` call starts a fresh subprocess, sends one JSON request on stdin, and reads one JSON response on stdout; stderr is diagnostic only. A provider describes the workflow and evaluates the exact transition selected by the engine. It cannot choose a different target or set current state. Check-free transitions do not invoke it.

## Agent rules

- Treat `initial_input` as immutable run configuration; never attempt to replace it.
- Treat context `kind` and `data` as opaque to Loop Engine core, but follow provider/state conventions for them; records are immutable and append-only.
- Use unique stable `--id` and `--record-id` values whenever an orchestrator controls identity.
- Request only an event shown for the current state, and request one at a time.
- Follow rejection feedback; append corrected evidence or steering before retrying the shown event.
- Do not progress final or terminated runs; `show` exposes no requestable events there.
- Use `history` for a semantic audit of creation, appends, transitions, checked-transition denials, and termination; other rejections (such as unavailable events or terminal mutations) are absent, and history is not an exhaustive execution trace.
