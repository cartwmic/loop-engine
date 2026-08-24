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

`--help` also lists `invocation-progress`, `fan-out`, and `preview-bindings` under Other commands, beside and distinct from these eight. They are not a ninth primary operation. `fan-out` and `preview-bindings` do not open the run database. `invocation-progress` opens the catalog; a query failure does not flip overlay.

```text
loop-engine [--database DB] [--json] [--timeout-ms MS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

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

Do not call `start` while any bound slot will invoke a model-bearing CLI (`pi --print`, `claude -p`, `run-plan-graph`'s inner worker, or similar) unless each model identifier is present in those frozen args, or the user has explicitly accepted that CLI's unpinned default as the model policy. An outer argv that does not name a model is not model lock-in. Do not bind a review slot when its configured policy-axis list is empty.

Provider review-binding constructors must read the same selected profile that will be passed to `start`. Software-change and research use their provider-specific per-slot `review_policies`; policy-document uses `semantic_policies`. A constructor normalizes missing `required_authors` to one, walks policies then the caller-confirmed roster in order, and freezes each worker's exact axis, provider-authored `example_prompt`, author claim, model, and provider-specific subject metadata. It requires pairwise-distinct non-empty author labels and non-empty models, and rejects unsupported or empty slots, missing axes/prompts/subject metadata, malformed or insufficient rosters, and invalid shipped worker-contract data.

The constructor atomically rewrites that same selected profile first. Only then may it extract and run `preview-bindings` on the resulting bindings, display the resulting profile's exact bytes and SHA-256, and obtain caller confirmation. Recheck that hash immediately before `start`, and start only that unchanged profile file. There is no post-preview merge. The frozen assignment is authoritative for a review worker; the later state instruction body is context only, not work for that reviewer.

If the user declines bindings, omit `work_slot_bindings` or set it to `{}`. A wrong freeze cannot be edited in place: terminate and start a new run.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: slot ID → `{command, args}`. Omit or `{}` means none. `start` rejects unknown slot IDs, unknown fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv.

`show` projects `work_slots` (catalog snapshot: id, state, event, optional `stdin_context_kinds`; no instruction body) and `work_slot_invocations`. Overlay status is `running` | `succeeded` | `failed` | `overrun`. Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). `show` does not spawn a provider and does not read capture files. Overlay meaning:

- succeeded: the bound CLI exited 0, not that the provider accepted the work
- failed: the bound CLI exited nonzero or the waiter vanished
- running: the waiter is alive and allowed time has not elapsed
- overrun: allowed time elapsed while the waiter is alive; run `show` immediately before re-invoking the same slot

Compare `current_state` to frozen bindings to decide the path:

- **Unbound** (cataloged or not, but absent from frozen bindings): `current_state_instructions` is the stored work body. Perform that work yourself. Then append and request the event.
- **Bound** (slot ID present in frozen bindings): instructions name the slot, the frozen `{command, args}`, and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`. They omit the stored work body. Do not perform that body, and do not exec the frozen command yourself. `invoke`, then poll `show` until overlay `succeeded`, `failed`, or `overrun`. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun run `show` immediately before re-invoking the same slot; on failed inspect `capture_dir/summary.json` and captured stdout before stderr.

`invoke RUN_ID SLOT_ID` starts the bound worker. Rejected for unknown, unbound, or overlay-`running` slots. Overlay `overrun` is terminal for retry, but re-read `show` immediately before re-invoking the same slot so a completed waiter does not cause redundant work. On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context` when the bound slot declared nonempty `stdin_context_kinds`. Fan-out framing adds no other packet keys. Hidden `wait-invocation` is parent of the worker; it is not a user command and not a daemon. After waitpid, if `capture_dir/summary.json` is well-formed, the waiter copies inner `command`/`args`/`exit_code` onto the invocation; overlay remains the bound CLI process exit (0 → succeeded). Hidden `stdin-exec` attaches `--stdin-file` to child stdin and runs `COMMAND [ARG]...` after `--` with no shell; duty bytes stay in that file, sidecar mode records `{"exit_code": <inner waitpid as i32>}` then exits 0, propagate mode is the inner waitpid, and `--help` omits it along with hidden `fan-out-join`. When `PI_CODING_AGENT_SESSION_DIR` is unset in the inherited environment, stdin-exec creates `<worker-capture-dir>/sessions` and sets that variable on the child only; do not add `--session-dir` to frozen argv, and do not switch bound Pi commands to `--mode json`. `software-change stdin-exec` is the duplicated provider helper with the same argv; plan-graph uses `--exit-mode propagate` only.

While overlay is `running`, the canonical driver poll is `show` (overlay, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, `inner_workers` empty) plus `loop-engine invocation-progress RUN_ID [INVOCATION_ID]` (`invocation_id`, `capture_dir`, per-step `not_started`|`running`|`reaped`, named sidecar/session traces). Graph state is Dagu helper liveness; `reaped` means the Dagu step helper finished, not overlay success and not inner waitpid 0. True inner waitpid remains in sidecar traces and `summary.json`; overlay remains the bound CLI process exit. A query failure does not flip overlay. `dagu status` / `dagu history` against the locator remain the underlying surface `invocation-progress` uses; they are not the driver-facing path.

`event`/`evaluate` never wait on a worker. A bound checked edge requires overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject.

For software-change runs, read the frozen `artifact_root/intent.json` `operating_context` before phase work. Preserve reviewer captures unchanged and let the driver, not a classifier or provider, append the authoritative `finding-ledger`; an `advisory-finding-proposal` never routes work or satisfies a gate. Review `full_output_schema` captures through `attempts.json` and both raw attempt directories: one same-worker retry is allowed, and exhaustion fails closed.

Implementation-ready and validation-ready are proof gates, not report declarations. The driver runs the provider's read-only `checkpoint` command against one existing absolute repository. It does not own Git lifecycle. If validation reports a checkpoint mismatch, request the shown check-free `revise-implementation` route, regenerate implementation and validation reports/checkpoints and fresh review evidence, then retry; do not silently substitute validation proof for implementation proof.

## Other command: invocation-progress

`invocation-progress` is listed with `fan-out` and `preview-bindings` under Other commands, not as a ninth primary. It opens the catalog, selects one invocation, and prints a JSON snapshot of that invocation's `capture_dir` graph liveness and already-associated traces. It does not write overlay. Skills carry this extra poll; overlay meaning is unchanged.

```text
loop-engine [--database DB] [--json] [--timeout-ms MS] invocation-progress RUN_ID [INVOCATION_ID]
```

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database. Each invocation emits a local Dagu `type:graph` under isolated `capture_dir/dagu-home/` (bound: packet `capture_dir`; ad-hoc: `cwd/fan-out-adhoc/<unique>`). Callers never supply Dagu YAML. The facade waitpids `dagu start --quiet --dagu-home` and does not daemonize. Drivers poll `show` for overlay; while overlay is `running`, poll `invocation-progress` for per-step graph state and named traces. Overlay remains the facade process exit. Dagu is GPLv3: invoke the operator-provided binary as a subprocess only; do not embed its Go API. Packages do not ship `dagu` (minimum 2.14.0 on PATH).

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
```

Workers come only from repeated strict nested `--worker` JSON objects. Each requires string `command` and array-of-string `args`; optional string `preamble`, legacy `output_schema`, and additive `full_output_schema` are allowed. The legacy schema syntax is `{"required":["key", ...]}` with at least one unique, non-empty top-level key name. The full field is a complete JSON Schema (or `{schema, retry_limit: 1}` wrapper), and its retry policy is fixed at one correction. Unknown or malformed nested fields fail closed. These fields do not extend the outer work-slot binding or nested `--task-worker`, which remain exactly `{command,args}`. Zero workers fail closed.

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Without `preamble`, worker stdin is byte-identical to the instruction-file bytes. With `preamble`, stdin is the preamble bytes unchanged, one LF appended only if the preamble does not already end in LF, literal `---\n\n`, then the unchanged instruction-file bytes (no `artifact_root` JSON). On success, stdout is exactly one JSON summary object (`dagu --quiet`). Re-invoke uses a new capture directory and a new home; prior captures are not overwritten.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot. Bound mode rejects `--instructions` and reads the existing worker packet (`run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context` when the slot declared nonempty `stdin_context_kinds`). Bound stdin does not dump `instruction_body`. Without `preamble`, including an `output_schema`-only worker, stdin is compact JSON with exactly absolute `artifact_root` plus one LF, and `context` when the invoke packet carried matching `stdin_context_kinds`. With `preamble`, stdin is exactly: preamble bytes unchanged; one LF only if absent; compact JSON serialized from `{"artifact_root": <absolute>}` plus `context` when forwarded; one LF; literal `---\n\n`; and no instruction body. The location object contains no `capture_dir` or duplicate run/slot identity, and adds no new invoke-packet keys or digest input.

Bound mode writes per-worker stdout/stderr under `packet.capture_dir/0/`, `1/`, … and writes `summary.json` in worker order. Worker steps `w<index>` start concurrently with no `continue_on` or `retry_policy`. Omitted `--max-active` emits no `max_active_steps` (uncapped); `--max-active N` is at most N worker steps. Hidden `stdin-exec --exit-mode sidecar` records the true inner waitpid; ordinary inner nonzero does not fail the facade. Mechanical `join` (`fan-out-join --capture-dir ABS`) depends on every worker, writes `summary.json`, invokes no model, and does not append `review-evidence`. If the graph stops before join, the facade still writes `summary.json` from spec and sidecars. True inner waitpid lives in the sidecar and `summary.json`; snapshot `reaped` is helper liveness. For `output_schema`, stdout must be a bare JSON object or the sole fenced `json` object amid prose; fan-out checks only required top-level key presence. For `full_output_schema`, the same frozen worker command, args, assignment, preamble, and model run at most twice. Attempt 2 keeps the original assignment and appends the unchanged attempt-1 stdout plus exact validation errors, asking only for schema-conforming reconsideration. Raw attempt bytes are under `<worker>/attempts/<N>/stdout` and `stderr`; `<worker>/attempts.json` stores schema version `1`, SHA-256 digests, validation errors, selected attempt, and exhaustion. On success compatibility stdout/stderr contain the selected attempt; the summary adds relative `attempts_path` and `selected_attempt` (`null` on exhaustion). A contracted summary entry preserves `command`, `args`, true `exit_code`, `stdout_path`, and `stderr_path`, adds `status` (`succeeded` or `failed`), and adds `conformance_error` only on failure. Workers without either schema preserve the legacy summary shape by omitting the contract fields. An exit-0 contract miss fails the facade after writing summary. Overlay success is not a review pass. Exit 0 and key presence do not establish semantic validity; the driver still validates and triages the values.

Shipped profiles omit `work_slot_bindings` (or `{}`), so slots stay driver-performed until opt-in. Use the relevant provider skill's deterministic review-binding constructor; do not hand-author one generic reviewer for a multi-axis gate, and do not bind a review slot whose configured policy-axis list is empty. Provider constructors freeze each worker's `preamble` and declared output contract inline before profile preview and lock-in; software-change review workers use assignment-specific `full_output_schema` constants. Bindings cannot be patched mid-run.

Pi templates keep `--no-skills --no-extensions` and add explicit `-e` paths so cursor-provider and claude-bridge load. Review workers include `--tools read,grep,find,ls` and must not pass `--no-context-files`. Implement workers do not add `--tools`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning.

Before rolling back to a pre-change binary, either keep a compatible binary available until every contracted run finishes, or terminate and restart each affected run without preamble or output_schema. This is operational guidance only and must not weaken immutable bindings or old-profile compatibility.

Opt-in implement example:

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--working-directory",
    "ABSOLUTE_EXISTING_DIRECTORY",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--model\",\"MODEL\"]}"
  ]
}
```

`software-change run-plan-graph` is an argv command of the software-change provider binary — not an engine operation. `--working-directory ABSOLUTE_EXISTING_DIRECTORY` is required and must be one driver-selected existing absolute directory frozen before `start`; it is applied to every ordinary plan task and the summarizer. Invalid or omitted values are rejected before any worker launches. Successful implementation graphs require the selected directory to be a Git working tree for checkpoint generation; the provider does not create, manage, select, or suggest worktrees. Bound mode honors `packet.capture_dir` (per-task `{task_id}/` plus `summary.json`). Each invocation emits a local Dagu `type:graph` under isolated `--dagu-home` at `capture_dir/dagu-home/`. Omitted `--max-active` is `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. The mandatory `summarizer` runs only after all ordinary tasks succeed; task failure leaves mechanical `summary.json` and captures and writes no `implementation-report.json`. The summarizer is the sole writer of `implementation-report.json`. Ordinary task stdin is compact `{"artifact_root"}` JSON plus that task's plan object only, with provider-added `finding_context` containing only exact-task current accepted unresolved implementation-owned ledger entries when ledger context is present. All ordinary tasks still run; proposals, stale/resolved/rejected/advisory, and unrelated entries are absent. While overlay is `running`, poll `invocation-progress` for per-step graph state and named traces; overlay remains the facade process exit. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths.

## Non-run-state command: preview-bindings

`preview-bindings` does not start, advance, or record a run and does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key.

It keeps outer bindings and nested `--task-worker` strict `{command,args}`, expands extended nested fan-out workers, reports `has_preamble`, legacy `output_schema.required`, and `full_output_schema`, and redacts preamble text from the printed binding argv. It lists detected `--model` values and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. It reports a `dagu` PATH check (minimum 2.14.0): ok with resolved path and version, or a warning naming the path or that PATH lookup found nothing. Warnings alone exit 0; `fan-out` and `software-change run-plan-graph` execute fail-close on the same condition before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). `dagu` is operator-provided and is not shipped in loop-engine or software-change packages. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

Repeat until `show` reports `final` or `terminated`:

1. `show` — read `current_state` (a state ID) with its sibling fields `current_state_title` and `current_state_instructions`, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations` (heartbeat fields above), ordered `context`, `requestable_events`, and `latest_evaluations` (includes latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` it and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`. While overlay is `running`, also poll `invocation-progress` for inner graph/traces (`inner_workers` stays empty on `show`). On overlay `overrun`, run `show` immediately before re-invoking. On failure, inspect `summary.json` and captured output before stderr. Otherwise follow the bound-instruction triage order. If unbound, perform the instructed work externally.
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
- Public-boundary proof uses `scripts/software-change-journey.py`; source full mode drives separate engine processes and packaged checked-prefix mode consumes only provider data materialized by `data-dump`. Policy-document and research journeys do the same for those providers. Each freezes a sparse dummy-worker `work_slot_bindings` entry and `invoke`s before the bound checked event. The software-change source full journey also proves `run-plan-graph` and `fan-out` with dummy inner workers (no live model), including `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, and prints `contracted fan-out failure` after bound deterministic workers prove the opted-in one-key `artifact_root` stdin contract and exit-0 nonconformance. `python3 scripts/software-change-journey.py --self-test` executes the three provider skill constructors and root AGENTS rules and prints `worker-data skill/root policy assertions passed` only after all pass. Synthetic evidence proves deterministic mechanics, not semantic verdict quality.
- `initial_input` is immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Do not `start` until the user has approved the bindings JSON, including default-vs-custom argv and models encoded in those args — or an explicit unpinned-default acceptance for any model-bearing CLI that has no model in argv.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination — not every read, provider failure, overlay `overrun`, or other rejection; history is not provider context.
