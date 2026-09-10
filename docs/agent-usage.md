# Agent usage

Use `loop-engine` to coordinate durable workflow state. Perform the primary work externally; the engine owns the run state, stored workflow graph, and progression decision. Read [PRD.md](PRD.md) for full semantics.

The staged [operational UX contracts](operational-ux-contracts.md) specify the
reviewed observation/capture/validation command forms and fixture ownership.
They are implementation contracts, not a claim that pending CLI cases work.
`loop-engine monitor` optionally accepts `--summary-config FILE --output-dir ABS`;
see the [advisory command and output contract](operational-ux-contracts.md#optional-advisory-summaries).
Summary subprocess failures and exhausted retained budgets never control workflow
state or stop deterministic observation. Reuse the output directory on restart.

## Deterministic setup

When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. That start stores the run in the user-level catalog and uses an engine-owned per-run artifact directory. This is the production start, not a usual-case option beside a prudent isolate alternative. Existing start examples that already omit both flags remain examples of this required start. Independent runs sharing the user-level catalog do not clobber each other, because each run already receives an engine-owned per-run artifact directory. Occupancy of the catalog by other runs, and fear of affecting those runs, are not reasons to pass `--database` or a nonempty `artifact_root`. An agent must not pass `--database` or a nonempty `artifact_root` unless the human explicitly asked to isolate in that session. Isolation is not a self-chosen precaution. `--database /path/to/dir/loop.db` isolates SQLite and `/path/to/dir/runs/<id>/`. A nonempty `artifact_root` isolates files to a caller-chosen absolute existing directory. Do not treat a prior session's isolation preference as standing authority.

When `--database` and database env vars are unset, the catalog is `$LOOP_ENGINE_HOME/loop.db` or `$LOOP_HOME/loop.db` if either home env is set, else `$XDG_DATA_HOME/loop-engine/loop.db` if `XDG_DATA_HOME` is set, else `$HOME/.local/share/loop-engine/loop.db`. `list` from any working directory reads that same file.

Pass `--json` and parse the single JSON envelope. Pass an explicit `--config` path to `start`; do not rely on path discovery.

That start stores the allocated absolute path in object `initial_input`. `start` may insert reserved `artifact_root` into object `initial_input` when the caller did not supply a nonempty path; object schemas that deny unknown keys must accept that field to remain evaluable; the engine does not skip injection, strip unknown keys, or classify providers. `list` JSON includes optional `provider` (the start alias) and `artifact_root`; `show` and `history` JSON keys are unchanged.

`--timeout-ms MILLISECONDS` is global and defaults to 30000 ms for each provider `describe` or `evaluate` call, and for `invoke` as the invocation's `allowed_time_ms`. Use an explicit timeout when provider latency or bound-worker runtime may exceed that default.

Provider TOML uses an exact, case-sensitive alias and a command invocation:

```toml
[providers.software-change]
command = "/absolute/path/to/provider"
args = []
```

`command` must be executable (or resolvable by the process environment). Keep `args` in the required order. The `start` initial input and `append` data accept JSON inline, from `@FILE`, or from `-` (stdin). These are JSON input sources only for those two operations; quote inline JSON for the shell.

## Command skeleton

Use the canonical executable name and these ten primary forms:

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MILLISECONDS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID [--view action|status|full] [--compact]
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID [--override JSON]
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invoke RUN_ID SLOT_ID [--preview] [--controls JSON] [--input DATA_JSON] [--assignment ID ... | --assignments ID,...]
loop-engine [--database DB] [--json] amend-binding RUN_ID SLOT_ID JSON
loop-engine [--database DB] [--json] cancel-invocation RUN_ID INVOCATION_ID
```

`--help` also lists `invocation-progress`, `fan-out`, and `preview-bindings` under Other commands, beside and distinct from these ten primary operations. `fan-out` and `preview-bindings` do not open the run database. `invocation-progress` opens the catalog; a query failure does not flip overlay.

```text
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

`show` defaults to `--view action`: current instructions, exact opaque persisted
provider guidance when available, bound/unbound execution paths, requestable
events, active ownership/captures, and source-located historical checks with
explicit unknown freshness. Historical checks are not fresh approval. Legacy
runs without guidance report normalized obligations as unknown, not zero;
follow persisted instructions and inspect the frozen input/context via full.
No show view invokes the provider. Core never interprets provider obligations.

`show --view full` includes complete initial input, context, bindings, invocation
and change reports, and every checked result in `evaluation_history` (original
transition, result/feedback, sequence and occurred_at). `latest_evaluations`
remains the latest-per-transition reduction. Overrides remain separate history,
not fabricated allows. Full-payload consumers, including provider commands fed
show JSON, must explicitly use `--view full`.

Action and full arm mutation for the current state visit. `show --view status`
and its `--compact` alias never arm mutation, in JSON or text. They do not revoke
an already valid observation. After a transition, read action/full before the
next mutation. JSON status exposes current state/control identity, sample time,
active work and uncertainty; omitted evidence has full/history locators.
The text status retains the fixed-order concise display and optional inner
progress. Missing graph/capture/Dagu data renders unavailable, never success.
`reaped` is Dagu helper completion, not worker success or provider acceptance.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

Global options may appear before or after the operation; the timeout option is shown once above to keep the skeleton brief. `start` resolves the provider alias and returns the run ID in `result.run.id`; supply `--id` when the orchestrator owns run identity. Supply `--record-id` for an orchestrator-owned context-record identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; it accepts opaque `KIND` and `DATA_JSON` and does not change state. Fresh selected-output evidence carries only `origin: {kind: "selected-assignment-output", id: INVOCATION_ID, assignment_id: ASSIGNMENT_ID}`. Core resolves that same-run invocation and assignment and adds the engine-owned selected attempt, capture, command, binding, path, and digest; do not copy those fields. Review reuse uses the distinct `evidence-applicability` kind with `{origin: {kind: "context-record", id: EVIDENCE_RECORD_ID}, target: {subject, revision, checkpoint}, attesting_driver, reason}`. The driver owns the semantic applicability claim; core checks only same-run context identity and the provider checks current software-change identities. Finding sources use the concise `context-record` reference. Historical completed-run records may remain readable, but legacy verbose linkage and carry forms are not new append paths.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: a sparse map from catalog slot ID to `{command, args, context_filter?}`. A filter is closed `{command,args}`. Omit the key or pass `{}` for no bindings. `start` rejects unknown slot IDs, unknown binding fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv. Bindings freeze with the run.

Lock that map with the user **before** `start`. Copying a provider profile is not approval. Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`). Bound slots are opt-in. Confirm whether any slots are bound, the exact `{command, args}` for each bound slot, and — when a bound CLI will invoke a model — which model identifiers are encoded in those frozen args. Nested inner workers (`--task-worker`, `--worker`) count. Do not call `start` while a bound model-bearing CLI has no model in argv unless the user has explicitly accepted that CLI's unpinned default. Do not bind a review slot when its configured policy-axis list is empty. Sparse: present keys are mandatory workers; absent keys stay driver-performed. Correct future execution with owner-attested `amend-binding`; do not patch initial input. Policy/topology changes are not binding corrections.

Use the provider skill's review-binding constructor. It must read the same selected profile that will be passed to `start`: software-change and research select their provider-specific per-slot `review_policies`, while policy-document selects `semantic_policies`. It normalizes absent `required_authors` to one and freezes exact assigned axes, prompts, author, model and subject metadata. Software-change assigns each policy to its first N roster entries and emits one batch per used author per gate; research and policy-document retain their existing per-policy construction. Ordinary/challenge gates stay separate. A justified software-change singleton uses the same batch shape. Author labels must be pairwise-distinct and non-empty, models must be non-empty, and unsupported/empty slots, missing axes/prompts/subject metadata, malformed or insufficient rosters, and invalid shipped worker-contract data must fail before start.

The constructor atomically rewrites that same selected profile first, then extracts its resulting bindings for `preview-bindings`, displays the resulting profile's exact bytes and SHA-256, and waits for caller confirmation. It rechecks the hash immediately before `start` and starts only that unchanged file; no post-preview merge is allowed. The frozen assignment is authoritative for review workers. Later state instructions are context only for them, not commands to perform driver work.

`show --view full` projects `work_slots` (catalog snapshot: id, state, event, optional `stdin_context_kinds`; no instruction body) and `work_slot_invocations` (including exact optional `invocation_input`, optional `assignment_selection`, and overlay status `running` | `succeeded` | `failed` | `overrun`). Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`assignment_id`, `command`, `args`, `exit_code`, plus selected-attempt identity/digest/originating path when selected, in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). Completed invocations project a durable provider-free `change_report` for assignment subject/assignment/binding/policy-configuration/output-contract/routed-input dimensions and for recorded plan-task definition/packet/dependencies/routed-input/worker-binding/task-recorded repository-effect dimensions. The public `change_report.assignments` key replaces the tombstoned `change_report.judgments` key with the same generic records; `plan_task_results` is unchanged. Standing assignments and plan-task results are visible from the run alone; unknown inputs are changed. `show` remains provider-free and reads engine-owned ownership/cancellation metadata under `capture_dir`; semantic interpretation of worker output remains the driver's duty. It is the observation that arms the current state visit for `append`, `event`, `invoke`, and `terminate`; after a state transition, show again before mutating. `list`, `history`, and `invocation-progress` do not arm mutation. Overlay meaning: succeeded means the bound CLI exited 0, not that the provider accepted the work; failed means the bound CLI exited nonzero or the waiter vanished; running means the waiter is alive and allowed time has not elapsed; overrun means allowed time elapsed while the waiter is alive; wait or cancel owned work and verify cleanup before retry. Missing waiter liveness alone is not proof of cleanup. When the current state is a bound slot, `current_state_instructions` names the slot ID plus the effective CLI binding (command, args and optional context_filter) and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it also says to consult the change report of record and use `evidence-applicability` for explicit review reuse. For unbound states, the same applicability guidance is appended to the stored instructions. The bound state omits the stored work body. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun wait or cancel, verify cleanup, then observe again before retry; on failed inspect `capture_dir/summary.json` and captured stdout before stderr. Unbound slots keep the stored instruction body and the driver-performed path.

After observing the current state with `show`, start bound work with `invoke RUN_ID SLOT_ID`. Optional `--input DATA_JSON` supplies one opaque JSON value to the bound executable; core preserves it exactly on the invocation and never interprets provider meaning. It cannot be combined with `--assignment` or `--assignments`. Add repeated `--assignment ID` or one `--assignments ID,...` to run only named enumerable fan-out assignments; omitted selection runs the full frozen binding. Empty, duplicate, unknown, or non-enumerable assignment selections are rejected before the waiter starts, and the validated selection is recorded on the invocation without changing frozen `work_slot_bindings`. Invoke is rejected for an unknown slot, an unbound slot, or an overlay-`running` invocation. Live owned work blocks retry regardless of elapsed allowance or waiter loss. Overrun is not terminal retry permission. Wait for completion or cancel and verify cleanup; inspect captures and observe again before retry. `event` and provider `evaluate` never wait on a worker.

On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional exact `invocation_input` from `--input`, optional `context` when the bound slot declared nonempty `stdin_context_kinds`, optional `assignment_selection` when invoke selected a fan-out subset, and optional `standing_assignment_ids` whenever context is forwarded. Standing IDs come from the same provider-free `show` projection; consumers that do not use them accept and ignore them. All software-change slots declare ledger, review-evidence, applicability, user-steering and steering-incorporation as eligible context; validation additionally forwards command, criterion, goal and revalidation records. The shared provider commission filter selects applicable recipients. Review assignments identify ordered axes and inspect current findings for each axis without changing policy or verdict authority. The ledger snapshots are ordinary immutable context records; `show` exposes their latest well-formed view and full append history. The implementation runner projects only current accepted unresolved implementation-owned entries routed to each exact task under `finding_context`; each selected task receives compact `{artifact_root, task}` stdin. Omitted task selection executes the full DAG; explicit roots execute those roots plus dependants after standing-prerequisite checks. The packet is not on argv, environment, or a temp file. Hidden `wait-invocation` is parent of that worker, waitpids it, and writes terminal `succeeded`/`failed`; after waitpid, a well-formed `capture_dir/summary.json` is copied as `inner_workers` (`command`, `args`, `exit_code`); overlay remains the bound CLI process exit (0 → succeeded). It is not a user command and not a daemon. A vanished waiter may project failed, but retained live ownership or pending cancellation still blocks retry/departure until verified cleanup.

Hidden `stdin-exec` attaches a file to a child stdin and runs `COMMAND [ARG]...` with no shell:

```text
loop-engine stdin-exec --stdin-file ABS --exit-mode sidecar|propagate [--sidecar-file ABS] -- COMMAND [ARG]...
```

Duty bytes live only in that file; they are not copied onto argv or into the child environment. Sidecar mode writes `{"exit_code": <inner waitpid as i32>}` to `--sidecar-file` (creating parent directories) after the child terminates, then the helper exits 0. Propagate mode uses the inner waitpid as the helper exit and rejects `--sidecar-file`. Spawn failure (missing binary, not executable) exits nonzero and does not write a successful sidecar. `--help` omits `stdin-exec` and `fan-out-join` the same way it omits `wait-invocation`. They are not user commands. When `PI_CODING_AGENT_SESSION_DIR` is already present in the inherited environment, stdin-exec leaves it unchanged. Otherwise, if `--stdin-file` has a parent directory, it creates `<parent>/sessions` and sets `PI_CODING_AGENT_SESSION_DIR` on the child only to that absolute path. Frozen worker argv is not rewritten and gains no `--session-dir`. Non-Pi children ignore the variable. Session traces that `invocation-progress` names live under that `sessions/` directory when the inner CLI honors the variable. Do not switch bound Pi commands to `--mode json`.

`software-change stdin-exec` is the same helper on the provider binary, duplicated in that crate rather than imported from `loop-cli`. Plan-graph uses `--exit-mode propagate` only so the helper exit is the inner waitpid; `--sidecar-file` is rejected in that mode. `software-change --help` and `--version` omit it.

A bound checked edge advances only after overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject. Overlay `running`/`failed`/`overrun` do not satisfy. Check-free edges omit bound-success/provider checks, but every departure requires quiescent owned work. Explicit override is separate below.

## Execution correction and cancellation

After `show`, use `amend-binding RUN SLOT @request.json` with the closed request `{"state_visit":4,"owner":"repository-owner","reason":"Correct future executor","binding":{"command":"/absolute/worker","args":[]}}`. Copy the actual `result.state_visit`, not this example integer. Binding may include `context_filter: {command,args}`. Unknown fields, stale visit, unknown slot or terminal run refuse. Initial input, policy, topology and previous invocations are unchanged; an active attempt keeps its original binding. `show.result.effective_bindings` exposes future settings beside original initial input.

`invoke RUN SLOT --preview --controls '{"max_active":1,"force_fresh":true}'` prepares the same effective binding, control and routed-context snapshot without creating an invocation/capture or primary worker. Launch recomputes under the single-driver rule. Controls are closed: positive max_active and boolean force_fresh; unsupported executable/control combinations refuse, not silently ignore them. Existing `--timeout-ms`, `--input` and assignment selection remain ordinary per-invocation settings. Fresh execution means new execution/captures without implicit standing reuse, not erasure of a worker CLI's private session. Force-fresh selected tasks cannot use unselected prerequisites as fresh; use a full run or normal standing-aware subset. Force-fresh review forbids carry.

`terminate RUN` refuses while invocation-owned work is live or cleanup is pending, including overrun. Wait for completion or cancel and verify cleanup, then `show` before terminating. Refusal leaves lifecycle and semantic history unchanged; terminated runs remain read-only.

`cancel-invocation RUN INVOCATION` controls only recorded local ownership for that run's current visit. It publishes a durable stop marker, stops Dagu/direct work, escalates and verifies disappearance/reaping before acknowledging terminal invocation failure. Available captures survive; the run does not advance or terminate. No later task or summarizer may start after cancellation admission. Wrong-run/nonrunning targets without an outstanding request refuse without mutation. Historical ownership absence produces an unsupported/ownership-unavailable refusal, not arbitrary-PID cancellation.

Each acquisition/resumption has one monotonic **ten-second** deadline, at most **three seconds** graceful shutdown and the rest escalation/verification. No phase resets the clock. The cancel CLI is the controller, not a daemon; it works from retained ownership even if the waiter died. If interrupted, the attempt stays incomplete and the live-work barrier remains. Already-running work may persist during an **unbounded operator-delay interval**; later task admission stays blocked. Repeat the same public command to resume that request with a newly recorded ten-second attempt. This is not a ten-second guarantee from the original request across interruption; timeout or unverified cleanup never becomes success retrospectively.

## Software-change evidence and recovery

Before any software-change phase, read the current `artifact_root/intent.json`. Its `operating_context` (`operators`, `environment`, `threat_boundary`, `accepted_risks`, and `outside_obligations`) is frozen for every later actor. Stay inside the trusted sole-operator boundary: do not add speculative hostile or multi-tenant demands, and never use an accepted risk to waive an outcome or outside obligation.

Reviewer output is candidate evidence. Preserve each raw response and append a driver-authored `finding-ledger` snapshot after triage. An `advisory-finding-proposal` is only a suggestion: edit or reject it as needed. It never satisfies a gate or changes a task packet. Current driver-accepted unresolved implementation findings with nonempty `task_ids` appear only in their exact tasks' `finding_context`. Omitted invocation input runs the full plan; exact `{plan_revision,task_roots}` input runs selected roots plus dependants after standing-prerequisite checks. If no frozen task honestly owns a current accepted unresolved implementation finding, leave its `task_ids` empty and use the same bound implement slot with exact `{repair_finding_ids}` input. Do not use repair for a task-owned defect, use a direct unbound substitute, or avoid a plan revision when decomposition is materially wrong. `show` is the fresh-actor view of the immutable ledger history, proposal records, exact invocation input, capture, and terminal outcome.

To inspect a completed bound review without changing the run, use the exact pipe:

```sh
"$ENGINE" --json show "$RUN_ID" --view full | "$PROVIDER" review-candidates
```

The provider returns a deterministic closed view in durable invocation/assignment order. `ready` means only mechanically verified selected bytes and contract fields (`axis`, `author`, `result`, and `findings`), with a stable invocation/assignment origin. `malformed`, `unavailable`, `missing-selection`, and `exhausted` are mechanical diagnostics without judgment fields. The command does not open the catalog, retry, deduplicate separate invocations, rewrite raw attempts, append, route, or satisfy a gate. Inspect `attempts.json` and raw captures, then explicitly accept, edit, or reject each candidate and use the ordinary `review-evidence` plus driver-authored `finding-ledger` append path before requesting the checked event. Candidate output is not evidence or semantic review.

For a bound `full_output_schema` reviewer, inspect `<capture_dir>/<worker>/attempts/<N>/stdout` and `stderr` plus `attempts.json`. The identical worker gets one correction attempt. Confirm the digests, exact validation errors, selected attempt, and `exhausted` flag before using the capture; exhaustion is failure, not a semantic verdict.

Implementation and validation reports are not checkpoints. For an unbound phase, after writing the report run:

```sh
software-change checkpoint --phase implementation \
  --artifact-root ABS_ARTIFACT_ROOT --working-directory ABS_EXISTING_REPOSITORY
software-change checkpoint --phase validation \
  --artifact-root ABS_ARTIFACT_ROOT --working-directory ABS_EXISTING_REPOSITORY
```

Both directories must already be absolute and existing. The command only reads Git and writes its phase checkpoint under `artifact_root`; it does not stage, commit, branch, push, create, select, merge, clean, or manage worktrees. The selected repository is the driver's responsibility. Checked implementation and validation events recompute the report, document, HEAD, index, status, tracked/non-ignored-untracked entries, and content identity. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, with or without implementation review. Validation requires the sole history entry for the current report revision to match the current report, document revisions, and repository state. Later context, regenerated mutable checkpoints, or overwritten history bytes do not replace that accepted-state anchor. If validation exposes a mismatch, request check-free `revise-implementation`, regenerate implementation report/checkpoint, regenerate validation report/checkpoint, re-run review evidence and the ledger, then retry the shown checked event. Do not replace a stale implementation checkpoint with a validation checkpoint.

## Other command: invocation-progress

`invocation-progress` is listed with `fan-out` and `preview-bindings` under Other commands, separate from the ten primary operations. Unlike those two, it opens the catalog. It does not append, invoke, request events, or write overlay. A failure or timeout of this query returns an error envelope and does not flip overlay or imply the waiter is unhealthy. `--timeout-ms` bounds helper spawns only, never invocation `allowed_time_ms`.

```text
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invocation-progress RUN_ID [INVOCATION_ID]
```

While overlay is `running`, the canonical driver poll is `show` plus `invocation-progress`, or the non-arming `show --compact` / `show --view status` view for a concise summary. `show` remains the overlay authority: overlay, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` empty. `invocation-progress` is the inner-progress document: it names `invocation_id`, `slot_id`, `capture_dir`, optional graph steps in `not_started`|`running`|`reaped` when `capture_dir/dagu-locator.json` is present, and already-associated sidecar or session traces (`path`, `kind` sidecar|session, `last_modified_ms`, optional step). Graph omitted means no locator yet or a non-graph bound CLI; `traces` is `[]` when none are found. The snapshot does not include overlay status, `inner_workers`, or worker stdout. Graph state is Dagu helper liveness: `reaped` means the Dagu step helper finished, not overlay success and not inner waitpid 0. True inner waitpid remains in named sidecar traces and later `summary.json`; overlay remains the bound CLI process exit. `dagu status` / `dagu history` against the locator remain the underlying surface `invocation-progress` uses; they are not the driver-facing path. Session traces live under a worker directory's `sessions/` subdirectory when hidden stdin-exec set `PI_CODING_AGENT_SESSION_DIR` there; do not add `--session-dir` to frozen argv, and do not switch bound Pi commands to `--mode json`.

When `INVOCATION_ID` is omitted, the unique overlay-running invocation is selected if one exists; otherwise the latest invocation by `started_at`. An early poll before the facade writes the locator can return `capture_dir` with graph omitted; retry while `show` still reports overlay `running`.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database. Each invocation emits a local Dagu `type:graph` only under an isolated `capture_dir/dagu-home/` (bound: `packet.capture_dir`; ad-hoc: `cwd/fan-out-adhoc/<unique>`). Callers never supply Dagu YAML. The facade waitpids `dagu start --quiet --dagu-home` and does not daemonize. Drivers poll `show` for overlay; while overlay is `running`, poll `invocation-progress` for per-step graph state and named traces. Overlay remains the facade process exit; Dagu is not a Loop Engine operation. Dagu is GPLv3: invoke the operator-provided binary as a subprocess only; do not embed its Go API. Packages do not ship `dagu` (minimum 2.14.0 on PATH).

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
```

Workers come only from repeated strict `--worker` JSON objects. Each requires string `command` and array-of-string `args`; it may also contain string `preamble`, legacy `output_schema` with exactly `{"required":["key", ...]}`, and additive `full_output_schema` containing a complete JSON Schema (the explicit `{schema, retry_limit: 1}` wrapper is also accepted). Unknown and malformed fields fail closed. `output_schema` remains required-key presence only for compatibility; `full_output_schema` validates the extracted JSON candidate against every declared JSON Schema constraint and gives the same worker one correction attempt, never a replacement worker. Its optional `x-loop-engine-force-fresh` annotation is an additional opaque JSON Schema constraint: bound `controls.force_fresh: true` adds it to `allOf` before launch, validates it and captures that effective schema without rewriting the binding. Software-change declares fresh-row requirements there so a reuse row cannot satisfy a force-fresh assignment. Other reference checks remain provider-owned; the engine does not learn review semantics. These are nested fan-out fields only: outer bindings accept `{command,args,context_filter?}`; nested task workers remain `{command,args}`.

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Without `preamble`, each worker receives byte-identical instruction-file bytes. With `preamble`, stdin is the preamble bytes, one LF appended only if absent, literal `---\n\n`, then the unchanged instruction-file bytes (no `artifact_root` JSON). Ad hoc mode has no invoke packet. On success it prints exactly one JSON summary object on stdout (`dagu --quiet`, no Dagu output interleaved). Zero `--worker` entries fail closed. Re-invoke uses a new capture directory, hence a new home and DAG/run names; prior captures are not overwritten.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot; `invoke` execs the frozen argv with the existing worker packet on stdin. Bound mode rejects `--instructions` and keeps that packet at `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context`, `assignment_selection`, and `standing_assignment_ids` supplied by invoke. Fan-out enforces selection and accepts but does not interpret generic standing IDs. Bound worker stdin no longer dumps `instruction_body`. Without `preamble`, including an `output_schema`-only worker, stdin is a compact JSON object with absolute `artifact_root` (and `context` when the invoke packet carried it) plus one LF. With `preamble`, stdin is exactly: preamble bytes; one LF only if absent; that same compact location JSON; one LF; literal `---\n\n`; and no instruction body. The location object has no `capture_dir` or duplicate run/slot identity.

Bound mode honors `packet.capture_dir` (writes per-worker `0/`, `1/`, … plus `summary.json` there, and `fan-out-spec.json` / `dagu-locator.json`). Worker steps `w<index>` start concurrently with no inter-worker depends. Omitted `--max-active` emits no `max_active_steps` (uncapped concurrent worker start); `--max-active N` is at most N worker steps. Each ordinary worker is `action:exec` of hidden `stdin-exec --exit-mode sidecar`; a `full_output_schema` worker uses the hidden same-worker retry runner instead. A mechanical `join` step depends on every worker, runs hidden `fan-out-join --capture-dir ABS`, writes `summary.json`, invokes no model, and does not append `review-evidence`. The graph does not set `continue_on` or `retry_policy`. If the graph stops before join, the facade still writes `summary.json` from spec and sidecars for every started worker. True inner waitpid lives in the sidecar and `summary.json`; snapshot `reaped` is helper liveness (helper exit 0 after the worker terminates). For `output_schema`, stdout must be either a bare JSON object or the sole fenced `json` object amid prose; malformed, missing, non-object, and ambiguous output fails. Fan-out checks only declared top-level key presence. For `full_output_schema`, the identical frozen worker command, args, assignment, preamble, and model run at most twice; the second assignment preserves the first assignment and includes the unchanged first stdout plus the exact validation errors, asking only for schema-conforming reconsideration. Raw bytes live at `<worker>/attempts/<N>/stdout` and `stderr`; `<worker>/attempts.json` has schema version `1`, per-attempt `sha256:<64 lowercase hex>` digests and errors, selected attempt, and exhaustion. On success the compatibility stdout/stderr paths contain the selected attempt. The summary adds the relative `attempts_path` and `selected_attempt` for this contract; exhaustion uses `null` and fails the facade. Every summary entry has `command`, `args`, true process `exit_code`, `stdout_path`, and `stderr_path`; contracted entries add `status` (`succeeded` or `failed`) and failure-only `conformance_error`, while uncontracted entries omit both new fields. Ordinary inner nonzero exits are recorded in the sidecar and do not fail the facade. An exit-0 contract miss fails the facade after writing summary. Overlay success is not a review pass. Exit 0 and key presence do not establish semantic deliverable validity.

Callers who want reviewers use the relevant provider skill's constructor to put assigned `--worker` objects in frozen binding args before `start`, `preview-bindings`, and lock-in. Future command/args/filter corrections require `amend-binding`; frozen policy and prior attempts never change. Do not bind a review slot whose configured policy-axis list is empty.

Shipped profiles omit `work_slot_bindings` (or `{}`), so slots stay driver-performed until opt-in. Review Pi workers keep `--no-skills --no-extensions`, add explicit `-e` paths, include `--tools read,grep,find,ls`, name their model, and must not pass `--no-context-files`. Implement workers do not add `--tools`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning.

Before rolling back to a pre-change binary, either keep a compatible binary available until every contracted run finishes, or terminate and restart each affected run without preamble or output_schema. This is operational guidance only and must not weaken immutable bindings or old-profile compatibility.

Opt-in implement example:

```json
"implement": {
  "command": "software-change",
  "args": [
    "run-plan-graph",
    "--working-directory",
    "/absolute/path/to/driver-selected-checkout",
    "--task-worker",
    "{\"command\":\"pi\",\"args\":[\"--print\",\"--no-skills\",\"--no-extensions\",\"-e\",\"CURSOR_EXTENSION_PATH\",\"-e\",\"CLAUDE_BRIDGE_EXTENSION_PATH\",\"--model\",\"MODEL\"]}"
  ]
}
```

`software-change run-plan-graph` is an argv command of the software-change provider binary — not an engine operation. For a bound `implement` slot, omitted invocation input runs the full plan; focused task-owned re-invocation uses `--input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'`. The provider requires that closed shape, the current plan revision, unique known roots, and every unselected prerequisite to have a successful standing result from that same revision; it then runs the roots plus their transitive dependants. If an accepted unresolved implementation finding has no honest frozen task owner, use the disjoint exact input `--input '{"repair_finding_ids":["FINDING_ID"]}'` on that same frozen slot after `show`. Every selected ID must resolve exactly once from current forwarded ledger, review-evidence, and applicability context, be implementation-owned, accepted, unresolved, current for the verified implementation checkpoint, and have empty `task_ids`. Malformed, empty, blank, duplicate, unknown, stale, wrong-owner/status/disposition, or task-routed requests refuse before Dagu resolution, stale report/checkpoint deletion, worker launch, or repository mutation. Do not combine packet input with frozen `--task`/`--tasks`.

A valid repair runs exactly one frozen worker as generic assignment `ad-hoc-repair`; it runs no plan task or summarizer and leaves `plan-task-results.json` unchanged. Its stdin carries `artifact_root` plus a closed repair assignment with the selected exact finding objects, frozen plan revision, provider-derived pre-report revision and pre-repository-state identity, and the obligation to make only the correction and write the new report. The worker must write a schema-valid `implementation-report.json` linked to the frozen plan with a revision unused by both the immediately preceding report and every accepted implementation-proof-history entry. The provider then creates a new implementation checkpoint. `capture_dir/summary.json` records the generic worker/output/routed-finding data and `repair` metadata with selected IDs and pre/post report and repository-state identities. If the worker fails or its report is invalid or colliding, inspect the capture and restore or deliberately incorporate any partial checkout changes before retry; no post checkpoint is created for those failures. Overlay success still is not semantic acceptance: append the resolved ledger snapshot and obtain fresh affected implementation review and validation proof. Task-owned defects use `{plan_revision,task_roots}`; materially wrong decomposition uses `revise-plan`. There is no direct unbound repair flag.

Direct `run-plan-graph` callers may use repeated `--task ID` (or one `--tasks ID,ID,...`); omitted selection remains full execution. Every invocation requires `--working-directory ABS`, where ABS is one existing absolute directory selected and maintained by the driver. Omitted, relative, nonexistent, and non-directory values are rejected before any Dagu graph worker starts. The selected directory is the graph-level cwd for every selected plan task and the summarizer, or for the single repair worker; successful execution requires it to be a Git working tree for checkpoint generation, and the provider does not create, discover, select, reuse, merge, clean, manage, or suggest worktrees. Bound mode honors `packet.capture_dir` (per-task or `ad-hoc-repair/` output plus `summary.json`). Each invocation emits a local Dagu `type:graph` under isolated `--dagu-home` at `capture_dir/dagu-home/` with fail-fast execution and no `continue_on`. Omitted `--max-active` is `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. While overlay is `running`, poll `invocation-progress` for per-step graph state and named traces; overlay remains the facade process exit. In full/selected mode a mandatory `summarizer` depends on every selected plan task, uses the frozen `--task-worker`, and is the sole writer of `artifact_root/implementation-report.json`. Ordinary task stdin is compact `{"artifact_root"}` JSON plus that task's plan object only, with an optional provider-added `finding_context` array containing only exact-task current accepted unresolved implementation-owned entries. Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths.

## Non-run-state command: preview-bindings

`preview-bindings` does not start, advance, or record a run and does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key. The outer binding is closed `{command,args,context_filter?}`; the filter and nested `--task-worker` remain closed `{command,args}`. Extended nested fan-out `--worker` entries report `has_preamble`, legacy `output_schema.required`, and `full_output_schema`; preview redacts the preamble text from the binding argv it prints. It lists detected `--model` values and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. JSON output includes a `dagu` check: ok with the PATH-resolved binary and version, or a warning naming the path (or that PATH lookup found nothing) and required version 2.14.0. Warnings alone exit 0. `fan-out` and `software-change run-plan-graph` execute fail-close on that same missing/unrunnable/unsupported-`dagu` condition before any worker spawn. Isolated home layout is `capture_dir/dagu-home/` with locator file `capture_dir/dagu-locator.json` holding exactly `dagu_home` (absolute), `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). `dagu` is operator-provided and is not shipped in loop-engine or software-change packages. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

For an active run, repeat this exact handoff loop:

1. Run `show`; inspect `current_state`, its title and instructions, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations` (heartbeat fields above), ordered `context`, `requestable_events`, and `latest_evaluations` (including latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` that slot and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`; while overlay is `running`, also poll `invocation-progress` for inner graph/traces (`inner_workers` stays empty on `show`). Do not perform the stored work body. On overlay `overrun`, wait or cancel owned work and verify cleanup, then run `show` before retry or departure. On failure, inspect `summary.json` and captured output before stderr. Follow the remaining bound-instruction triage order. Otherwise perform the instructed work externally. Do not expect the engine to edit the repository, document, or other external work.
3. Append durable context for useful evidence, findings, decisions, or steering. Context is opaque to Loop Engine core, not necessarily to the provider or workflow; follow provider/state conventions for `kind` and `data`. Core assigns no truth, provenance, approval, or supersession meaning. Every checked evaluation receives immutable `initial_input` and all accumulated context in stable append order.
4. Select one event from `requestable_events` and request exactly that `event`. Append any final handoff context before requesting an event that enters a final state.
5. Inspect the JSON status, then run `show` again before selecting another event. On `rejected`, follow its feedback and continue work; on `error`, do not assume anything advanced and re-read `show` (use `history` when auditing).
6. Stop when `show` reports `final` or `terminated`; these runs are read-only: `append`, `event`, and `terminate` are rejected without semantic history, and no requestable progression is exposed.

Request events, never target states. The engine resolves the target from the run's stored workflow. Request only events returned by the latest `show`; an event valid elsewhere in the graph is unavailable from the current state.

### Explicit exceptional progression

An owner-attested exception uses the current `show.result.state_visit`, a nonempty owner and reason, and exactly one available event:

```sh
loop-engine --json event RUN_ID implementation-ready --override \
  '{"state_visit":4,"owner":"repository-owner","reason":"Accept this specific missing completion proof as an exception"}'
```

The closed JSON request also accepts `@FILE`; unknown fields, stale visits, unavailable events and terminal runs refuse without override history. `show` is still required. Live, elapsed-but-live, and cancellation-cleanup-pending work still blocks: wait or use `cancel-invocation`, then observe again. This is trusted owner testimony, not cryptographic authorization or permission to leave writers running.

Only the named edge's bound-completion and provider-evaluation obligations are skipped. Its history outcome is `overridden`, with the attestation, exact transition, known skipped bound check (slot, instruction digest, current visit subject), and `provider_evaluation: "not-performed"` (`"not-applicable"` on a check-free edge). There is no synthetic provider allow, internal check inventory, artifact, review pass, or Bookends GREEN. Earlier denials, failed evidence, and RED/BYPASS output remain unchanged. A later ordinary edge still needs its own proof or its own separately attested exception. In software-change, skipping implementation admission can leave no accepted `implementation-proof-history` entry. A regenerated mutable checkpoint does not fill that gap, and downstream validation still refuses missing history. Restore genuine proof through supported owning-phase work or seek a separate owner-attested exception; never fabricate the skipped provider's historical success.

`show`, `list` rows, event-result runs and human output expose `has_overrides`, `override_count`, and `completion_mode`, derived permanently from history. The JSON `history` envelope exposes these same fields beside its unchanged `result` array. Active/terminated runs have null completion mode. Ordinary final runs have `false`, `0`, `"completed"`; exceptional final runs have `true`, a positive count, `"completed-with-overrides"`. Lifecycle remains `final`; operation `status: "completed"` alone does not mean exception-free workflow success. Provider-free reads retain these distinctions even when the provider executable is unavailable. Historical graphs gain no new edges, and absent historical override entries mean zero exceptions—not new cancellation capability.

## JSON results

With `--json`, an operation returns one envelope. A successful operation includes `operation`, `status: "completed"`, and `result`; for example, `{"operation":"list","status":"completed","result":[]}`. An understood-but-denied request has `status: "rejected"` plus `code`, `message`, and optional `details`. An operation that cannot be reliably evaluated or committed has `status: "error"` with the same issue fields. Malformed CLI syntax or input returns only `status`, `code`, and `message`—no `operation`, `result`, or `details`:

```json
{"status":"invalid-invocation","code":"invalid-invocation","message":"missing required event ID"}
```

Exit codes map to `0` completed, `10` rejected, `20` error, and `2` invalid invocation. Parse the JSON even on a nonzero exit. Treat only `completed` as operation success. Never infer state advancement from `rejected` or `error`; re-run `show` against the same database. A checked-transition provider denial is `rejected` and exposes durable actionable feedback. Provider failure or `unsupported` is `error` and does not authorize progression. `history` includes checked-transition denials and work-slot invocation started/status-changed actions, not every read, provider failure, overlay `overrun`, or diagnostic; other rejections, such as unavailable events or terminal mutations, are absent.

## Handoff and provider boundary

`show` is a provider-free durable handoff: it reads the database and does not spawn or query the provider. A fresh agent needs the run ID, access to the same database, and access to every external reference carried in the initial input, context, or state instructions. It does not need the previous conversation or raw history to resume normal work. History is not context and is not supplied wholesale to provider evaluation; use it for semantic audit.

At `start`, the provider alias resolves once to its configured `command` and ordered `args`; that association, plus the provider's workflow topology and state-instruction snapshot, is stored with the run. Changing TOML cannot redirect an existing run. Executable contents at the stored command path may change; a later provider implementation evaluates the stored workflow snapshot and may return `unsupported` if it no longer supports it. Each provider `describe` or checked-transition `evaluate` call starts a fresh subprocess, sends one JSON request on stdin, and reads one JSON response on stdout; stderr is diagnostic only. A provider describes the workflow and evaluates the exact transition selected by the engine. It cannot choose a different target or set current state. Check-free transitions do not invoke it.

## Generate-PRD research path

Repositories without a schema-valid living PRD can use the existing research provider with `crates/research-provider/data/configs/generate-prd.json` and `crates/research-provider/skills/using-generate-prd/SKILL.md`. The deterministic source journey writes `prd-candidate.md` and an evidence sidecar, reaches research `end`, and runs `bookends-check candidate` against the candidate. Candidate IDs are provisional: a human must accept or reject the candidate before any commit to `docs/PRD.md`; the path does not edit or commit that file.

## Software-change Bookends overlay

The overlay is off by default. On a per-run copy of a shipped software-change profile, set `extra.bookends.enabled` to JSON `true`; do not edit shipped profiles. It adds exactly one `prd_traceability` disposition to each current intent criterion: `linked-live` with nonempty live `LE-<n>` IDs, a parser-valid `candidate` with a proposed `LE-<n>`, or `not-applicable` with a short reason. Missing or tombstoned linked IDs and malformed candidates are denied; a current candidate blocks final completion for that Bookends-enabled run, while `not-applicable` never waives or fulfills its criterion. Validation `passed` is refused on checker `RED` or `BYPASS`. Downstream artifacts use only optional current-intent `AC-N` references; the provider adds `ids-grounded` and validation `bypass-not-green` review axes.

At each durable `e2e/journey` or declared `contract` test boundary, the driver or bound worker cites the same live ID as `bookends:LE-<n>` in the captured/test output. The driver triages that output and appends evidence. The repository gate command is `scripts/bookends-check-gate.sh`; it prints `GREEN`, `RED`, or `BYPASS`, and accepts only `BOOKENDS_BYPASS=<class>:<reason>` as explicit invocation evidence. Parser-only candidate validation is `bookends-check candidate PRD.md`. README.md and AGENTS.md are not coverage classes.

## Agent rules

- One logical mutating actor per run: serialize `append`, `event`, `invoke`, and `terminate` calls; never race them from parallel workers. Concurrent reads are fine. Context appended during an in-flight checked evaluation does not invalidate or reach that evaluation.
- Treat `initial_input` as immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Confirm that map with the user before `start`, including models in frozen args or an explicit unpinned-default acceptance.
- Treat context `kind` and `data` as opaque to Loop Engine core, but follow provider/state conventions for them; records are immutable and append-only.
- Use unique stable `--id` and `--record-id` values whenever an orchestrator controls identity. Append accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged in append results, `show` context, and durable `history`.
- Request only an event shown for the current state, and request one at a time.
- Follow rejection feedback; append corrected evidence or steering before retrying the shown event. Fresh bound evidence names only its invocation/assignment origin; review reuse uses one `evidence-applicability` record with a current target, attesting driver, and short reason. Finding sources are immutable `context-record` references.
- Do not progress final or terminated runs; `show` exposes no requestable events there.
- Use `history` for a semantic audit of creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination; other rejections (such as unavailable events or terminal mutations) are absent, overlay `overrun` is not a history action, and history is not an exhaustive execution trace.

## Contract v2 software-change continuation

New shipped profiles declare `contract_version: 2`, versions `minimal-9`, `standard-9`, `high-rigor-9`, and `criterion_policy: {required_authors:1,goal_required_authors:1}` independent of axis counts. They remain unbound. Unsupported earlier semantic profiles refuse describe/evaluate explicitly (provider nonzero/stderr, engine error 20 without advancement). Bare describe is discovery. Keep fixed original binaries for old-profile execution; new binaries do not migrate input, stored topology or old invocation ownership. Provider-free show/history preserve original evidence, not newly proven capability.

Inspect steering with `loop-engine --json show RUN --view full | software-change commission --slot SLOT [--task TASK]`. User-steering targets `all`, named `slots`, or `{kind:"tasks",plan_revision,ids}`; explicit supersedes replaces whole records in append order. Task-only instructions do not spread to dependants/summarizer, stale task revisions are reported and excluded, and launched attempts keep their snapshot. Configure `context_filter:{command:"/absolute/software-change",args:["commission"]}` for bound recipient selection. The engine admits only ordered existing record IDs, never rewritten packets. Proof updates name an existing proof_commands ID, owner and/or argv, and a reason, preserving the obligation. Unbound steering-incorporation records attest what was applied.

Current review failures need exact-source reasoned rejection/resolution or valid retired-author disposition. Discharged fails count as independent judgments, not passes. Accepted-unresolved findings still block across revision bumps. Reviewer-manifest history must show departure for retirement; retired authors do not count and replacement coverage is required. Historical resolved sources need no false applicability. See the software-change reviewer protocol for exact records.

The final report is an index, not freehand proof. `loop-engine --json show RUN --view full | software-change run-validation --engine ABS --working-directory ABS --revision REV [--commands ID,...]` executes named plan proof_commands and writes an index plus inert command candidates. It neither appends nor advances; inspect/append candidates, finish and checkpoint the index before actual criterion/goal judgments. Selection never waives unselected required proof. Exactly one selected verdict set per current AC-N plus a separate goal judgment must satisfy independent criterion_policy at approval (or reviewless final draft hop). Ordinary axes consume that collection; challenge does not duplicate it. After repair declare affected criteria, supply fresh verdicts there and explicitly carry unaffected sources with reason/current checkpoint. Material repair requires fresh goal judgment. Missing, stale, duplicate, unknown, self-authored or unresolved failing coverage blocks.

Use the software-change skill and shipped validation template for full record forms. Policy-document and research gain the common engine controls, not software-change-specific steering, disposition, grouped review or criterion contracts. Simple-first/YAGNI/KISS apply to all providers; meaningful current need and inadequate simpler alternatives belong in the ordinary design, not a new justification framework.

## Workspace Rust test and preflight path

The ordinary full Rust suite uses the exact `cargo-nextest` version pinned in
`.config/nextest.toml`:

```sh
cargo install cargo-nextest --version 0.9.143 --locked
python3 scripts/assert-nextest.py
python3 scripts/run-nextest.py
```

`run-nextest.py` is the fresh-binary build contract: it builds the five
production/reference packages and all three fixture provider binaries from the
current source, hands their direct `target/debug` paths to the central suite
through `LOOP_ENGINE_TEST_BINARY_HANDOFF`, runs workspace unit tests and the
single `workspace-integration/workspace` target through nextest, and then runs
owning-crate doctests with Cargo. Focused iteration is
`python3 scripts/run-nextest.py --filter TEST_SUBSTRING`; it skips doctests.
The deliberately wedged `process_timeout_probe` is not part of the ordinary
selection; prove its configured timeout and descendant cleanup with
`python3 scripts/prove-nextest-timeout.py --report /tmp/nextest-timeout.json`.
Do not use workspace-wide `--test-threads=1`.

The central target contract can be inspected without executing its tests:

```sh
python3 scripts/run-central-tests.py \
  --no-run \
  --compiler-artifacts /tmp/central-test-artifacts.jsonl \
  --handoff-output /tmp/stock-cargo-handoff.json
python3 scripts/assert-test-topology.py \
  --compiler-artifacts /tmp/central-test-artifacts.jsonl
python3 scripts/assert-test-inventory.py \
  --compiler-artifacts /tmp/central-test-artifacts.jsonl \
  --current-only
```

The topology assertion is based on Cargo metadata and compiler-artifact
messages and requires exactly one integration-test executable across the
workspace. The inventory assertion maps every declaration in the central
source modules to the names emitted by that executable; the predecessor
baseline and central-harness artifact provide the before/after equivalence
proof. The explicit handoff is current-build evidence, not a search fallback:
it rejects missing, non-executable, outside-target, digest-mismatched, and
hashed `target/debug/deps` candidates.

Stock compatibility remains an independent final gate and does not route
through `run-central-tests.py`. Use the fresh handoff produced above:

```sh
test -s /tmp/stock-cargo-handoff.json
LOOP_ENGINE_TEST_BINARY_HANDOFF=/tmp/stock-cargo-handoff.json \
  cargo test --workspace
```

Run Cargo consumers serially against one checkout. The repository dependency
audit is similarly pinned and reproducible:

```sh
cargo install cargo-machete --version 0.9.2 --locked
python3 scripts/dependency-audit.py
```

For local compiler-cache proof, use the credential-free disk backend and the
same real-Rust two-target check; the proof creates no repository cache or
`target` data and never runs `cargo clean`:

```sh
python3 scripts/sccache-proof.py --self-test
python3 scripts/sccache-proof.py proof \
  --artifact /tmp/testing-sccache.json \
  --work-root /tmp/loop-engine-sccache-proof
```

Required GitHub-hosted preflight installs sccache 0.17.0 through
`mozilla-actions/sccache-action@v0.0.11`, exports the ephemeral
`ACTIONS_RESULTS_URL` and `ACTIONS_RUNTIME_TOKEN` values required by its
credential-free GHA backend, starts it before any Cargo compilation, and emits
startup and final statistics; no repository secret is used.
It also installs nextest and cargo-machete at the pins above, runs the
ordinary nextest/doctest path, timeout/structure/inventory/audit assertions,
the direct stock Cargo gate, clippy/fmt, locked package builds, Bookends,
generated-release checks, and the four public journeys as separate serialized
steps. `.github/workflows/release.yml` remains cargo-dist-generated; only the
hand-authored `preflight.yml` is changed for this contract.

The final stable-revision matrix is driver-owned and serialized in this
order under the current plan-owned matrix: tool/cache setup, dependency/release/Bookends checks, nextest/doctests, timeout/topology/inventory, stock Cargo, clippy/fmt/builds, discovery/profile/interface checks, every required source journey, required final benchmark collect/compare, then local cache statistics and total wall time. Workers perform assigned focused validation; reviewers consume retained results. The designated proof owner repeats only checks invalidated by later changes. The journey's one `--jobs` budget defaults to 2 for independent isolated work, with serial 1; shared-run hops and outer phases remain ordered. Hosted exact-commit statistics/preflight and later real dogfood remain separate pending acts. Use [implementation-report-proof.md](implementation-report-proof.md) for current matrix receipts and the non-circular report checker. A local focused pass or a static workflow assertion is not a
claim that this final matrix or the hosted sccache run passed.

## Executable proof boundaries

Repository checks name boundaries they actually cross:

- component tests cover parser, core, SQLite, provider schema/evidence, and protocol behavior;
- composed tests combine engine operations, SQLite, and provider subprocesses below CLI process boundary;
- `scripts/software-change-journey.py --mode source --traversal-depth full` drives separate `loop-engine` processes through provider TOML, SQLite, production `software-change` process, copied high-rigor artifacts, sparse dummy-worker `work_slot_bindings`, `invoke` before the bound checked event, deterministic denials, evidence aggregation, and terminal state. It also proves frozen operating-context forwarding, proposal inertness, driver ledger routing, concise invocation/assignment origins with engine-resolved capture metadata, immutable context-record finding sources, one current evidence-applicability declaration, missing/wrong-run/stale/non-current identity denials, raw-attempt preservation, invalid-then-valid retry, invalid-twice exhaustion, report-only checkpoint denial, every named repository-state invalidation against implementation and validation proof, validation recovery, and final current-tree outcome proof in real temporary Git repositories. It proves unbound shipped profiles, `software-change run-plan-graph` and `loop-engine fan-out` with dummy inner workers (no live model), compact `artifact_root` stdin with no `instruction_body` dump, `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, and `preview-bindings` warning when pi has `--no-extensions` and no `-e`. The bound plan-graph implement journey freezes a driver-owned symlink alias, requires the checkout's `.git` marker, and checks every task and summarizer cwd with filesystem-equivalence semantics. The plan-graph dummy writes `implementation-report.json` only when stdin is the summarizer assignment. Bound contracted workers capture stdin and emit conforming JSON or exit-0 refusal text; after the compact one-key `artifact_root` context, failed overlay, and persisted summary/captures are proven, it prints `contracted fan-out failure`. The named Package 7b scenario pipes ordinary `show` into `software-change review-candidates`, proves selected retry, exhausted diagnostics, raw-capture preservation, deterministic repeated inspection, inertness before driver records, and progression afterward. CI preflight installs operator-provided `dagu` 2.14.0 onto PATH before those tests and does not copy the binary into dist artifacts. `python3 scripts/software-change-journey.py --self-test` runs the three provider skill constructors against high-rigor design-review, policy-document shipped semantic policies/target/mode, and research verify plus synthesize, asserts root AGENTS rules, and prints `worker-data skill/root policy assertions passed` only after all pass;
- `scripts/software-change-journey.py --mode packaged --traversal-depth checked-prefix` consumes extracted binaries, calls `data-dump`, and runs release-critical checked transition from dumped data only, including the same sparse binding/`invoke` proof;
- `scripts/policy-document-journey.py` drives draft and audit modes through provider TOML, SQLite, production `policy-document` process, a sparse dummy-worker binding on `semantic-review`, ungated `prepare` → `ready`, and deterministic/semantic denials;
- `scripts/research-journey.py --mode source` drives separate `loop-engine` processes through provider TOML, SQLite, production `research` process, copied standard profile, sparse dummy-worker `work_slot_bindings`, `invoke` before `scoped`, schema/evidence denials, and terminal state;
- `scripts/research-journey.py --mode packaged` consumes extracted binaries, calls `data-dump`, and runs the release-critical checked prefix from dumped data only, including the same sparse binding/`invoke` proof.

Journey evidence records are synthetic, schema-conforming pass records. They prove deterministic policy mechanics, author independence, revision-link handling, routing, aggregation, and persistence; they do not prove semantic review or verdict quality. Semantic review remains external.
