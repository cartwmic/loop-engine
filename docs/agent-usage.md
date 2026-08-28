# Agent usage

Use `loop-engine` to coordinate durable workflow state. Perform the primary work externally; the engine owns the run state, stored workflow graph, and progression decision. Read [PRD.md](PRD.md) for full semantics.

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

Use the canonical executable name and these eight primary forms:

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MILLISECONDS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show [--compact] RUN_ID
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invoke RUN_ID SLOT_ID [--input DATA_JSON] [--assignment ID ... | --assignments ID,...]
```

`--help` also lists `invocation-progress`, `fan-out`, and `preview-bindings` under Other commands, beside and distinct from these eight. They are not a ninth primary operation. `fan-out` and `preview-bindings` do not open the run database. `invocation-progress` opens the catalog; a query failure does not flip overlay.

```text
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

`show --compact RUN_ID` is a command-local, human-only projection of the
ordinary `show` result. It prints, in fixed order, the run identity and
lifecycle, current state, requestable events, latest checked result, the
active invocation (or latest invocation), and inner progress. The invocation
line is sourced from `show` and remains the overlay authority; graph progress
never changes its `running`/`succeeded`/`failed`/`overrun` status or exit code.
Compact deliberately omits `show`'s observation-time `elapsed_ms` and
`remaining_allowed_ms` counters, so repeated reads of unchanged state have the
same text; detailed JSON `show` remains authoritative for those fields. The
inner line counts `not_started`, `running`, and `reaped` Dagu helper states when
the existing `invocation-progress` collector can provide a graph.
A missing graph, missing capture data, missing Dagu executable, or collector
failure is rendered as `inner progress: unavailable [...] ...`; a valid durable
`show` therefore still exits successfully and no task state is invented. With
`--json`, use ordinary `show` for the complete projection or
`invocation-progress` for its detailed inner-progress document. Combining
`--compact` with JSON output is rejected rather than defining a second JSON
schema. `reaped` means only that a Dagu step helper finished; it is not worker
success or provider acceptance.

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

Global options may appear before or after the operation; the timeout option is shown once above to keep the skeleton brief. `start` resolves the provider alias and returns the run ID in `result.run.id`; supply `--id` when the orchestrator owns run identity. Supply `--record-id` for an orchestrator-owned context-record identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; it accepts opaque `KIND` and `DATA_JSON` and does not change state. The reserved optional `originating_output` object is the sole generic identity exception: it must exactly match an engine-selected `{invocation_id, assignment_id, selected_attempt, sha256, path}` from `show`; core never reads or interprets the bytes. The existing append path also has two explicit carry acts: use `KIND=unchanged-carry` or `KIND=override-carry` with JSON `{source_record_id?, invocation_id, assignment_id, attesting_driver, overridden_inputs?}`; omit `source_record_id` when carrying a plan-task result that has no context record. The engine copies the originating record, reads the change report of record, refuses unchanged-carry when any covered input changed, and requires override-carry to name every changed input. The result and `show` identify the act, attesting driver, original selected-output digest, and overridden inputs; carry output names the change report and both acts. A carry stands only while the exact dimension snapshot it attested still matches; later drift requires another explicit carry. Do not supply a cleanliness flag or treat a carry as a semantic decision by the engine.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: a sparse map from catalog slot ID to `{command, args}`. Omit the key or pass `{}` for no bindings. `start` rejects unknown slot IDs, unknown binding fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv. Bindings freeze with the run.

Lock that map with the user **before** `start`. Copying a provider profile is not approval. Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`). Bound slots are opt-in. Confirm whether any slots are bound, the exact `{command, args}` for each bound slot, and — when a bound CLI will invoke a model — which model identifiers are encoded in those frozen args. Nested inner workers (`--task-worker`, `--worker`) count. Do not call `start` while a bound model-bearing CLI has no model in argv unless the user has explicitly accepted that CLI's unpinned default. Do not bind a review slot when its configured policy-axis list is empty. Sparse: present keys are mandatory workers; absent keys stay driver-performed. A wrong freeze cannot be patched; terminate and start a new run.

Use the provider skill's review-binding constructor. It must read the same selected profile that will be passed to `start`: software-change and research select their provider-specific per-slot `review_policies`, while policy-document selects `semantic_policies`. It normalizes absent `required_authors` to one; expands policies then a caller-confirmed roster in order; and freezes each worker's exact axis, provider-authored `example_prompt`, author claim, model, and provider-specific subject metadata. Author labels must be pairwise-distinct and non-empty, models must be non-empty, and unsupported/empty slots, missing axes/prompts/subject metadata, malformed or insufficient rosters, and invalid shipped worker-contract data must fail before start.

The constructor atomically rewrites that same selected profile first, then extracts its resulting bindings for `preview-bindings`, displays the resulting profile's exact bytes and SHA-256, and waits for caller confirmation. It rechecks the hash immediately before `start` and starts only that unchanged file; no post-preview merge is allowed. The frozen assignment is authoritative for review workers. Later state instructions are context only for them, not commands to perform driver work.

`show` projects `work_slots` (catalog snapshot: id, state, event, optional `stdin_context_kinds`; no instruction body) and `work_slot_invocations` (including exact optional `invocation_input`, optional `assignment_selection`, and overlay status `running` | `succeeded` | `failed` | `overrun`). Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`assignment_id`, `command`, `args`, `exit_code`, plus selected-attempt identity/digest/originating path when selected, in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). Completed invocations project a durable provider-free `change_report` for assignment subject/assignment/binding/policy-configuration/output-contract/routed-input dimensions and for recorded plan-task definition/packet/dependencies/routed-input/worker-binding/task-recorded repository-effect dimensions. The public `change_report.assignments` key replaces the tombstoned `change_report.judgments` key with the same generic records; `plan_task_results` is unchanged. Standing assignments and plan-task results are visible from the run alone; unknown inputs are changed. `show` does not spawn a provider and does not read capture files. It is the observation that arms the current state visit for `append`, `event`, `invoke`, and `terminate`; after a state transition, show again before mutating. `list`, `history`, and `invocation-progress` do not arm mutation. Overlay meaning: succeeded means the bound CLI exited 0, not that the provider accepted the work; failed means the bound CLI exited nonzero or the waiter vanished; running means the waiter is alive and allowed time has not elapsed; overrun means allowed time elapsed while the waiter is alive; run `show` immediately before re-invoking the same slot. When the current state is a bound slot, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it also says to consult the change report of record and distinguishes unchanged-carry from override-carry. For unbound states, the same carry guidance is appended to the stored instructions. The bound state omits the stored work body. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun run `show` immediately before re-invoking the same slot; on failed inspect `capture_dir/summary.json` and captured stdout before stderr. Unbound slots keep the stored instruction body and the driver-performed path.

After observing the current state with `show`, start bound work with `invoke RUN_ID SLOT_ID`. Optional `--input DATA_JSON` supplies one opaque JSON value to the bound executable; core preserves it exactly on the invocation and never interprets provider meaning. It cannot be combined with `--assignment` or `--assignments`. Add repeated `--assignment ID` or one `--assignments ID,...` to run only named enumerable fan-out assignments; omitted selection runs the full frozen binding. Empty, duplicate, unknown, or non-enumerable assignment selections are rejected before the waiter starts, and the validated selection is recorded on the invocation without changing frozen `work_slot_bindings`. Invoke is rejected for an unknown slot, an unbound slot, or an overlay-`running` invocation. Overlay `overrun`, `failed`, and `succeeded` are not already-running. Overlay `overrun` is terminal for retry, but the driver must run `show` immediately before re-invoking the same slot so a completed waiter does not cause redundant work. `event` and provider `evaluate` never wait on a worker.

On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional exact `invocation_input` from `--input`, optional `context` when the bound slot declared nonempty `stdin_context_kinds`, optional `assignment_selection` when invoke selected a fan-out subset, and optional `standing_assignment_ids` whenever context is forwarded. Standing IDs come from the same provider-free `show` projection; consumers that do not use them accept and ignore them. Software-change review slots and the implementation slot declare `finding-ledger`; other draft slots omit the field. Review assignments identify one axis and inspect current entries whose `review_axes` include it without changing reviewer policy or verdict authority. The ledger snapshots are ordinary immutable context records; `show` exposes their latest well-formed view and full append history. The implementation runner projects only current accepted unresolved implementation-owned entries routed to each exact task under `finding_context`; each selected task receives compact `{artifact_root, task}` stdin. Omitted task selection executes the full DAG; explicit roots execute those roots plus dependants after standing-prerequisite checks. The packet is not on argv, environment, or a temp file. Hidden `wait-invocation` is parent of that worker, waitpids it, and writes terminal `succeeded`/`failed`; after waitpid, a well-formed `capture_dir/summary.json` is copied as `inner_workers` (`command`, `args`, `exit_code`); overlay remains the bound CLI process exit (0 → succeeded). It is not a user command and not a daemon. A vanished waiter with no terminal status is overlay-`failed`.

Hidden `stdin-exec` attaches a file to a child stdin and runs `COMMAND [ARG]...` with no shell:

```text
loop-engine stdin-exec --stdin-file ABS --exit-mode sidecar|propagate [--sidecar-file ABS] -- COMMAND [ARG]...
```

Duty bytes live only in that file; they are not copied onto argv or into the child environment. Sidecar mode writes `{"exit_code": <inner waitpid as i32>}` to `--sidecar-file` (creating parent directories) after the child terminates, then the helper exits 0. Propagate mode uses the inner waitpid as the helper exit and rejects `--sidecar-file`. Spawn failure (missing binary, not executable) exits nonzero and does not write a successful sidecar. `--help` omits `stdin-exec` and `fan-out-join` the same way it omits `wait-invocation`. They are not user commands. When `PI_CODING_AGENT_SESSION_DIR` is already present in the inherited environment, stdin-exec leaves it unchanged. Otherwise, if `--stdin-file` has a parent directory, it creates `<parent>/sessions` and sets `PI_CODING_AGENT_SESSION_DIR` on the child only to that absolute path. Frozen worker argv is not rewritten and gains no `--session-dir`. Non-Pi children ignore the variable. Session traces that `invocation-progress` names live under that `sessions/` directory when the inner CLI honors the variable. Do not switch bound Pi commands to `--mode json`.

`software-change stdin-exec` is the same helper on the provider binary, duplicated in that crate rather than imported from `loop-cli`. Plan-graph uses `--exit-mode propagate` only so the helper exit is the inner waitpid; `--sidecar-file` is rejected in that mode. `software-change --help` and `--version` omit it.

A bound checked edge advances only after overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject. Overlay `running`/`failed`/`overrun` do not satisfy. Check-free edges are ungated.

## Software-change evidence and recovery

Before any software-change phase, read the current `artifact_root/intent.json`. Its `operating_context` (`operators`, `environment`, `threat_boundary`, `accepted_risks`, and `outside_obligations`) is frozen for every later actor. Stay inside the trusted sole-operator boundary: do not add speculative hostile or multi-tenant demands, and never use an accepted risk to waive an outcome or outside obligation.

Reviewer output is candidate evidence. Preserve each raw response and append a driver-authored `finding-ledger` snapshot after triage. An `advisory-finding-proposal` is only a suggestion: edit or reject it as needed. It never satisfies a gate or changes a task packet. Current driver-accepted unresolved implementation findings with nonempty `task_ids` appear only in their exact tasks' `finding_context`. Omitted invocation input runs the full plan; exact `{plan_revision,task_roots}` input runs selected roots plus dependants after standing-prerequisite checks. If no frozen task honestly owns a current accepted unresolved implementation finding, leave its `task_ids` empty and use the same bound implement slot with exact `{repair_finding_ids}` input. Do not use repair for a task-owned defect, use a direct unbound substitute, or avoid a plan revision when decomposition is materially wrong. `show` is the fresh-actor view of the immutable ledger history, proposal records, exact invocation input, capture, and terminal outcome.

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

`invocation-progress` is listed with `fan-out` and `preview-bindings` under Other commands, not as a ninth primary. Unlike those two, it opens the catalog. It does not append, invoke, request events, or write overlay. A failure or timeout of this query returns an error envelope and does not flip overlay or imply the waiter is unhealthy. `--timeout-ms` bounds helper spawns only, never invocation `allowed_time_ms`.

```text
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invocation-progress RUN_ID [INVOCATION_ID]
```

While overlay is `running`, the canonical driver poll is `show` plus `invocation-progress`, or the human-only `show --compact` view for a concise summary. `show` remains the overlay authority: overlay, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` empty. `invocation-progress` is the inner-progress document: it names `invocation_id`, `slot_id`, `capture_dir`, optional graph steps in `not_started`|`running`|`reaped` when `capture_dir/dagu-locator.json` is present, and already-associated sidecar or session traces (`path`, `kind` sidecar|session, `last_modified_ms`, optional step). Graph omitted means no locator yet or a non-graph bound CLI; `traces` is `[]` when none are found. The snapshot does not include overlay status, `inner_workers`, or worker stdout. Graph state is Dagu helper liveness: `reaped` means the Dagu step helper finished, not overlay success and not inner waitpid 0. True inner waitpid remains in named sidecar traces and later `summary.json`; overlay remains the bound CLI process exit. `dagu status` / `dagu history` against the locator remain the underlying surface `invocation-progress` uses; they are not the driver-facing path. Session traces live under a worker directory's `sessions/` subdirectory when hidden stdin-exec set `PI_CODING_AGENT_SESSION_DIR` there; do not add `--session-dir` to frozen argv, and do not switch bound Pi commands to `--mode json`.

When `INVOCATION_ID` is omitted, the unique overlay-running invocation is selected if one exists; otherwise the latest invocation by `started_at`. An early poll before the facade writes the locator can return `capture_dir` with graph omitted; retry while `show` still reports overlay `running`.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database. Each invocation emits a local Dagu `type:graph` only under an isolated `capture_dir/dagu-home/` (bound: `packet.capture_dir`; ad-hoc: `cwd/fan-out-adhoc/<unique>`). Callers never supply Dagu YAML. The facade waitpids `dagu start --quiet --dagu-home` and does not daemonize. Drivers poll `show` for overlay; while overlay is `running`, poll `invocation-progress` for per-step graph state and named traces. Overlay remains the facade process exit; Dagu is not a Loop Engine operation. Dagu is GPLv3: invoke the operator-provided binary as a subprocess only; do not embed its Go API. Packages do not ship `dagu` (minimum 2.14.0 on PATH).

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
```

Workers come only from repeated strict `--worker` JSON objects. Each requires string `command` and array-of-string `args`; it may also contain string `preamble`, legacy `output_schema` with exactly `{"required":["key", ...]}`, and additive `full_output_schema` containing a complete JSON Schema (the explicit `{schema, retry_limit: 1}` wrapper is also accepted). Unknown and malformed fields fail closed. `output_schema` remains required-key presence only for compatibility; `full_output_schema` validates the extracted JSON candidate against every declared JSON Schema constraint and gives the same worker one correction attempt, never a replacement worker. These are nested fan-out fields only: the outer work-slot binding remains exactly `{command,args}`.

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Without `preamble`, each worker receives byte-identical instruction-file bytes. With `preamble`, stdin is the preamble bytes, one LF appended only if absent, literal `---\n\n`, then the unchanged instruction-file bytes (no `artifact_root` JSON). Ad hoc mode has no invoke packet. On success it prints exactly one JSON summary object on stdout (`dagu --quiet`, no Dagu output interleaved). Zero `--worker` entries fail closed. Re-invoke uses a new capture directory, hence a new home and DAG/run names; prior captures are not overwritten.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot; `invoke` execs the frozen argv with the existing worker packet on stdin. Bound mode rejects `--instructions` and keeps that packet at `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context`, `assignment_selection`, and `standing_assignment_ids` supplied by invoke. Fan-out enforces selection and accepts but does not interpret generic standing IDs. Bound worker stdin no longer dumps `instruction_body`. Without `preamble`, including an `output_schema`-only worker, stdin is a compact JSON object with absolute `artifact_root` (and `context` when the invoke packet carried it) plus one LF. With `preamble`, stdin is exactly: preamble bytes; one LF only if absent; that same compact location JSON; one LF; literal `---\n\n`; and no instruction body. The location object has no `capture_dir` or duplicate run/slot identity.

Bound mode honors `packet.capture_dir` (writes per-worker `0/`, `1/`, … plus `summary.json` there, and `fan-out-spec.json` / `dagu-locator.json`). Worker steps `w<index>` start concurrently with no inter-worker depends. Omitted `--max-active` emits no `max_active_steps` (uncapped concurrent worker start); `--max-active N` is at most N worker steps. Each ordinary worker is `action:exec` of hidden `stdin-exec --exit-mode sidecar`; a `full_output_schema` worker uses the hidden same-worker retry runner instead. A mechanical `join` step depends on every worker, runs hidden `fan-out-join --capture-dir ABS`, writes `summary.json`, invokes no model, and does not append `review-evidence`. The graph does not set `continue_on` or `retry_policy`. If the graph stops before join, the facade still writes `summary.json` from spec and sidecars for every started worker. True inner waitpid lives in the sidecar and `summary.json`; snapshot `reaped` is helper liveness (helper exit 0 after the worker terminates). For `output_schema`, stdout must be either a bare JSON object or the sole fenced `json` object amid prose; malformed, missing, non-object, and ambiguous output fails. Fan-out checks only declared top-level key presence. For `full_output_schema`, the identical frozen worker command, args, assignment, preamble, and model run at most twice; the second assignment preserves the first assignment and includes the unchanged first stdout plus the exact validation errors, asking only for schema-conforming reconsideration. Raw bytes live at `<worker>/attempts/<N>/stdout` and `stderr`; `<worker>/attempts.json` has schema version `1`, per-attempt `sha256:<64 lowercase hex>` digests and errors, selected attempt, and exhaustion. On success the compatibility stdout/stderr paths contain the selected attempt. The summary adds the relative `attempts_path` and `selected_attempt` for this contract; exhaustion uses `null` and fails the facade. Every summary entry has `command`, `args`, true process `exit_code`, `stdout_path`, and `stderr_path`; contracted entries add `status` (`succeeded` or `failed`) and failure-only `conformance_error`, while uncontracted entries omit both new fields. Ordinary inner nonzero exits are recorded in the sidecar and do not fail the facade. An exit-0 contract miss fails the facade after writing summary. Overlay success is not a review pass. Exit 0 and key presence do not establish semantic deliverable validity.

Callers who want reviewers use the relevant provider skill's constructor to put assigned `--worker` objects in frozen binding args before `start`, `preview-bindings`, and lock-in. Bindings cannot be patched mid-run. Do not bind a review slot whose configured policy-axis list is empty.

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

`software-change run-plan-graph` is an argv command of the software-change provider binary — not an engine operation. For a bound `implement` slot, omitted invocation input runs the full plan; focused task-owned re-invocation uses `--input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'`. The provider requires that closed shape, the current plan revision, unique known roots, and every unselected prerequisite to have a successful standing result from that same revision; it then runs the roots plus their transitive dependants. If an accepted unresolved implementation finding has no honest frozen task owner, use the disjoint exact input `--input '{"repair_finding_ids":["FINDING_ID"]}'` on that same frozen slot after `show`. Every selected ID must resolve exactly once from current forwarded `finding-ledger` context, be implementation-owned, accepted, unresolved, current for the verified implementation checkpoint, and have empty `task_ids`. Malformed, empty, blank, duplicate, unknown, stale, wrong-owner/status/disposition, or task-routed requests refuse before Dagu resolution, stale report/checkpoint deletion, worker launch, or repository mutation. Do not combine packet input with frozen `--task`/`--tasks`.

A valid repair runs exactly one frozen worker as generic assignment `ad-hoc-repair`; it runs no plan task or summarizer and leaves `plan-task-results.json` unchanged. Its stdin carries `artifact_root` plus a closed repair assignment with the selected exact finding objects, frozen plan revision, provider-derived pre-report revision and pre-repository-state identity, and the obligation to make only the correction and write the new report. The worker must write a schema-valid `implementation-report.json` linked to the frozen plan with a revision unused by both the immediately preceding report and every accepted implementation-proof-history entry. The provider then creates a new implementation checkpoint. `capture_dir/summary.json` records the generic worker/output/routed-finding data and `repair` metadata with selected IDs and pre/post report and repository-state identities. If the worker fails or its report is invalid or colliding, inspect the capture and restore or deliberately incorporate any partial checkout changes before retry; no post checkpoint is created for those failures. Overlay success still is not semantic acceptance: append the resolved ledger snapshot and obtain fresh affected implementation review and validation proof. Task-owned defects use `{plan_revision,task_roots}`; materially wrong decomposition uses `revise-plan`. There is no direct unbound repair flag.

Direct `run-plan-graph` callers may use repeated `--task ID` (or one `--tasks ID,ID,...`); omitted selection remains full execution. Every invocation requires `--working-directory ABS`, where ABS is one existing absolute directory selected and maintained by the driver. Omitted, relative, nonexistent, and non-directory values are rejected before any Dagu graph worker starts. The selected directory is the graph-level cwd for every selected plan task and the summarizer, or for the single repair worker; successful execution requires it to be a Git working tree for checkpoint generation, and the provider does not create, discover, select, reuse, merge, clean, manage, or suggest worktrees. Bound mode honors `packet.capture_dir` (per-task or `ad-hoc-repair/` output plus `summary.json`). Each invocation emits a local Dagu `type:graph` under isolated `--dagu-home` at `capture_dir/dagu-home/` with fail-fast execution and no `continue_on`. Omitted `--max-active` is `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. While overlay is `running`, poll `invocation-progress` for per-step graph state and named traces; overlay remains the facade process exit. In full/selected mode a mandatory `summarizer` depends on every selected plan task, uses the frozen `--task-worker`, and is the sole writer of `artifact_root/implementation-report.json`. Ordinary task stdin is compact `{"artifact_root"}` JSON plus that task's plan object only, with an optional provider-added `finding_context` array containing only exact-task current accepted unresolved implementation-owned entries. Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths.

## Non-run-state command: preview-bindings

`preview-bindings` does not start, advance, or record a run and does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key. The outer binding and nested `--task-worker` remain strict `{command,args}`. Extended nested fan-out `--worker` entries report `has_preamble`, legacy `output_schema.required`, and `full_output_schema`; preview redacts the preamble text from the binding argv it prints. It lists detected `--model` values and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. JSON output includes a `dagu` check: ok with the PATH-resolved binary and version, or a warning naming the path (or that PATH lookup found nothing) and required version 2.14.0. Warnings alone exit 0. `fan-out` and `software-change run-plan-graph` execute fail-close on that same missing/unrunnable/unsupported-`dagu` condition before any worker spawn. Isolated home layout is `capture_dir/dagu-home/` with locator file `capture_dir/dagu-locator.json` holding exactly `dagu_home` (absolute), `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). `dagu` is operator-provided and is not shipped in loop-engine or software-change packages. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

For an active run, repeat this exact handoff loop:

1. Run `show`; inspect `current_state`, its title and instructions, immutable `initial_input` (including `work_slot_bindings` when present), `work_slots`, `work_slot_invocations` (heartbeat fields above), ordered `context`, `requestable_events`, and `latest_evaluations` (including latest checked-transition denial feedback).
2. If the current state is a bound slot, `invoke` that slot and poll `show` until overlay status is `succeeded`, `failed`, or `overrun`; while overlay is `running`, also poll `invocation-progress` for inner graph/traces (`inner_workers` stays empty on `show`). Do not perform the stored work body. On overlay `overrun`, run `show` immediately before re-invoking the same slot. On failure, inspect `summary.json` and captured output before stderr. Follow the remaining bound-instruction triage order. Otherwise perform the instructed work externally. Do not expect the engine to edit the repository, document, or other external work.
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

## Generate-PRD research path

Repositories without a schema-valid living PRD can use the existing research provider with `crates/research-provider/data/configs/generate-prd.json` and `crates/research-provider/skills/using-generate-prd/SKILL.md`. The deterministic source journey writes `prd-candidate.md` and an evidence sidecar, reaches research `end`, and runs `bookends-check candidate` against the candidate. Candidate IDs are provisional: a human must accept or reject the candidate before any commit to `docs/PRD.md`; the path does not edit or commit that file.

## Software-change Bookends overlay

The overlay is off by default. On a per-run copy of a shipped software-change profile, set `extra.bookends.enabled` to JSON `true`; do not edit shipped profiles. It requires nonempty live `LE-<n>` IDs in `intent.json`, `design.json`, `plan.json`, and `validation-report.json`, rejects missing or tombstoned IDs, and calls the same checker in-process without a repository bypass. Validation `passed` is refused on checker `RED` or `BYPASS`. The provider adds only `ids-grounded` and validation `bypass-not-green` review axes.

At each durable `e2e/journey` or declared `contract` test boundary, the driver or bound worker cites the same live ID as `bookends:LE-<n>` in the captured/test output. The driver triages that output and appends evidence. The repository gate command is `scripts/bookends-check-gate.sh`; it prints `GREEN`, `RED`, or `BYPASS`, and accepts only `BOOKENDS_BYPASS=<class>:<reason>` as explicit invocation evidence. Parser-only candidate validation is `bookends-check candidate PRD.md`. README.md and AGENTS.md are not coverage classes.

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
- `scripts/software-change-journey.py --mode source --traversal-depth full` drives separate `loop-engine` processes through provider TOML, SQLite, production `software-change` process, copied high-rigor artifacts, sparse dummy-worker `work_slot_bindings`, `invoke` before the bound checked event, deterministic denials, evidence aggregation, and terminal state. It also proves frozen operating-context forwarding, proposal inertness, driver ledger routing, raw-attempt preservation, invalid-then-valid retry, invalid-twice exhaustion, report-only checkpoint denial, every named repository-state invalidation against implementation and validation proof, validation recovery, and final current-tree outcome proof in real temporary Git repositories. It proves unbound shipped profiles, `software-change run-plan-graph` and `loop-engine fan-out` with dummy inner workers (no live model), compact `artifact_root` stdin with no `instruction_body` dump, `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, and `preview-bindings` warning when pi has `--no-extensions` and no `-e`. The bound plan-graph implement journey freezes a driver-owned symlink alias, requires the checkout's `.git` marker, and checks every task and summarizer cwd with filesystem-equivalence semantics. The plan-graph dummy writes `implementation-report.json` only when stdin is the summarizer assignment. Bound contracted workers capture stdin and emit conforming JSON or exit-0 refusal text; after the compact one-key `artifact_root` context, failed overlay, and persisted summary/captures are proven, it prints `contracted fan-out failure`. CI preflight installs operator-provided `dagu` 2.14.0 onto PATH before those tests and does not copy the binary into dist artifacts. `python3 scripts/software-change-journey.py --self-test` runs the three provider skill constructors against high-rigor design-review, policy-document shipped semantic policies/target/mode, and research verify plus synthesize, asserts root AGENTS rules, and prints `worker-data skill/root policy assertions passed` only after all pass;
- `scripts/software-change-journey.py --mode packaged --traversal-depth checked-prefix` consumes extracted binaries, calls `data-dump`, and runs release-critical checked transition from dumped data only, including the same sparse binding/`invoke` proof;
- `scripts/policy-document-journey.py` drives draft and audit modes through provider TOML, SQLite, production `policy-document` process, a sparse dummy-worker binding on `semantic-review`, ungated `prepare` → `ready`, and deterministic/semantic denials;
- `scripts/research-journey.py --mode source` drives separate `loop-engine` processes through provider TOML, SQLite, production `research` process, copied standard profile, sparse dummy-worker `work_slot_bindings`, `invoke` before `scoped`, schema/evidence denials, and terminal state;
- `scripts/research-journey.py --mode packaged` consumes extracted binaries, calls `data-dump`, and runs the release-critical checked prefix from dumped data only, including the same sparse binding/`invoke` proof.

Journey evidence records are synthetic, schema-conforming pass records. They prove deterministic policy mechanics, author independence, revision-link handling, routing, aggregation, and persistence; they do not prove semantic review or verdict quality. Semantic review remains external.
