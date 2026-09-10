---
name: using-loop-engine
description: Use when driving a durable Loop Engine workflow run from the CLI — confirming work-slot policy with the user before start, previewing bindings, starting a run against a configured provider, inspecting or resuming a run, appending context records, requesting events, invoking bound work slots, terminating, interpreting completed/rejected/error/invalid-invocation outcomes, or fanning out worker CLIs without a run.
---

# Using Loop Engine

## Overview

`loop-engine` owns durable run state, the stored workflow graph, and the progression decision. You perform the primary work externally; the engine never edits repositories, documents, or artifacts. Workflows come from external provider executables configured in TOML. Provider `describe` and `evaluate` are deterministic and do not invoke a model.

This skill is the engine source of truth. Reference provider skills embed a driving minimum and name this skill as a required companion; they do not replace it.

Full semantics: repository `docs/agent-usage.md`. Requirements: repository `docs/PRD.md`. Simple-first, YAGNI and KISS apply to the engine and every provider; explain meaningful current complexity in the ordinary design, not a new justification framework.

## Deterministic setup

When the human did not explicitly ask to isolate in that session, omit `--database` and omit `artifact_root`. Production uses the user-level catalog and an engine-owned per-run artifact directory. Other runs sharing that catalog do not clobber these files and do not justify isolation; neither does a prior session's preference. Only an explicit current-session request permits `--database /path/to/dir/loop.db` (SQLite and `/path/to/dir/runs/<id>/`) or a nonempty `artifact_root` (caller-chosen absolute existing directory).

Before start, inspect the operation's CLI flags, profile `artifact_root`, and environment overrides; resolve and record the actual catalog path. Unset unintended database redirects in that operation's environment. An inherited redirect is not human isolation permission; do not erase intentional home configuration indiscriminately.

Catalog precedence is explicit `--database`, then the first nonempty value among `LOOP_ENGINE_DATABASE`, `LOOP_ENGINE_DATABASE_PATH`, `LOOP_DATABASE`, `LOOP_DATABASE_PATH`, `LOOP_DB_PATH`, `LOOP_ENGINE_DB`, `LOOP_DB`, in that order. Otherwise use `LOOP_ENGINE_HOME/loop.db`, then `LOOP_HOME/loop.db`, then `XDG_DATA_HOME/loop-engine/loop.db`, then `$HOME/.local/share/loop-engine/loop.db` (`USERPROFILE` when HOME is absent). Tilde expansion follows path selection. `list` from any working directory reads that same resolved catalog.
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

## Confirm profile and external fleet before start

For a software-change run, confirm two separate authorities immediately before `start`: the exact selected per-run profile file and, when work is unbound, an owner-confirmed external role-to-model manifest. The profile supplies `config_version`, every nonempty live review state and axis, normalized `required_authors` (missing means one), Bookends enabled/disabled state, and frozen `work_slot_bindings` plus any nested model arguments. Derive and display those facts from the same file passed as `@$PROFILE`; print its exact bytes and SHA-256, run `preview-bindings` on its exact bindings, obtain explicit owner confirmation, and rehash that file immediately before `start`. Never substitute a shipped default or claim the profile hash covers unbound assignments.

Keep the external manifest separate from the profile and bindings, with generic per-run entries such as `{"ordinary-reviewer":"MODEL_ID","challenge-reviewer":"MODEL_ID"}` plus every other role actually launched (for example, analyst, writer, or implementer). Print its exact bytes and SHA-256 and obtain separate owner confirmation. For each unbound launch, run `pi --list-models`, require the exact confirmed model ID, pass that ID in the launch arguments, and preserve evidence of the role, manifest hash, and actual model argument. If the exact model cannot launch or differs from the manifest, stop; never fall back, substitute, or silently use a default. With all slots unbound, this manifest governs external launches while the profile still governs workflow obligations.

## Commands

```text
loop-engine [--database DB] [--config CONFIG] [--json] [--timeout-ms MS] start [--id RUN_ID] PROVIDER INITIAL_JSON [LABEL]
loop-engine [--database DB] [--json] list
loop-engine [--database DB] [--json] show RUN_ID [--view action|status|full] [--compact]
loop-engine [--database DB] [--json] append [--record-id RECORD_ID] RUN_ID KIND DATA_JSON
loop-engine [--database DB] [--json] event RUN_ID EVENT_ID [--override JSON]
loop-engine [--database DB] [--json] history RUN_ID
loop-engine [--database DB] [--json] terminate RUN_ID
loop-engine [--database DB] [--json] [--timeout-ms MS] invoke RUN_ID SLOT_ID [--preview] [--controls JSON] [--input DATA_JSON] [--assignment ID ... | --assignments ID,...]
loop-engine [--database DB] [--json] amend-binding RUN_ID SLOT_ID JSON
loop-engine [--database DB] [--json] cancel-invocation RUN_ID INVOCATION_ID
```

Other commands include `invocation-progress`, `fan-out`, `preview-bindings`, `monitor`, and the capture commands below. `fan-out`, `preview-bindings`, and capture do not open the run database. `invocation-progress` opens the catalog; a query failure does not flip overlay.

```text
loop-engine [--database DB] [--json] [--timeout-ms MS] invocation-progress RUN_ID [INVOCATION_ID]
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
loop-engine preview-bindings [JSON|@FILE]
```

```sh
loop-engine --json --config /absolute/path/to/providers.toml \
  start software-change @/tmp/profile.json "my run"
```

`start` returns the run ID at `result.run.id`; supply `--id` when the orchestrator owns run identity. `append` accepts both `--record-id VALUE` and `--record-id=VALUE`; supplied IDs remain unchanged through result, `show`, and `history`. Append is opaque to provider meaning and never changes state. Fresh selected-output evidence uses only `origin: {kind: "selected-assignment-output", id: INVOCATION_ID, assignment_id: ASSIGNMENT_ID}`. Core resolves that same-run invocation and assignment and adds engine-owned selected attempt, capture, command, binding, path, and digest metadata; do not copy those fields. Review reuse uses the distinct `evidence-applicability` context kind with `{origin: {kind: "context-record", id: EVIDENCE_RECORD_ID}, target: {subject, revision, checkpoint}, attesting_driver, reason}`. The driver owns semantic applicability; core checks same-run context identity and the provider checks current software-change identities. Finding sources use the concise context-record reference. Historical completed-run records remain readable, but legacy verbose linkage and carry forms are not new append paths.

## Choose an observation

| Need | Command | Arms the current visit? |
|---|---|---|
| Act or resume driving | `loop-engine --json show RUN --view action` (default) | Yes |
| Inspect status once | `loop-engine --json show RUN --view status` (`--compact` alias) | No |
| Read frozen input, complete context/history, change reports or helper stdin | `loop-engine --json show RUN --view full` | Yes |
| Wait for execution or attention | `loop-engine monitor --run RUN --json --attention-seconds 300` | No |

Action is the focused handoff: read current instructions, available events, blockers and active-work locators. Use full when a consumer needs omitted fields; do not feed action/status to helpers expecting full show. Missing normalized obligations on a legacy graph mean unknown, not zero authors. After a transition, read action/full again before mutation. Status, monitor, list, history and invocation-progress do not arm a visit.

### Monitor instead of model polling

Run one passive observer while work proceeds. Put `monitor` first; its `--json` output is **JSONL**, not the workflow envelope. Repeat `--run` or `--capture-dir ABS` for independent sources; `--invocation ID` scopes one selected run. For a released engine lacking status views, use `monitor --engine /absolute/released/loop-engine --run RUN --json`; its restricted compatibility reads never arm that run.

A deterministic consumer should notify the driver on a selected source's `completion` or `attention`, preserving `source`, `boundary.reason` and evidence locators. The monitor keeps following until interrupted; stop the observer when no longer needed, not the observed work. Inspect workflow, helper, worker, conformance and judgment lanes separately. Neither completion nor an elapsed attention deadline approves, retries or cancels anything. Re-read actionable show before acting. Use `invocation-progress RUN [INVOCATION]` for specific graph/trace diagnosis: helper `reaped` is not inner exit 0 or provider acceptance.

Optional summaries require owner-approved command/model and spend. Add `--summary-config FILE --output-dir ABS`; without configuration there are no model calls. Read repository `docs/operational-ux-contracts.md`, **Optional advisory summaries**, for the exact command stdin/output and config. Configure timeout, minimum interval and maximum calls; keep the same output directory on restart to preserve budget/cadence. Retained prose is advisory, may be stale, and never supplies gate evidence or control authority.

## Capture external commands

Use common capture for authorized external proof commands, not to bypass a bound slot's `invoke` path. Choose an existing absolute working directory and an output directory outside the measured checkout. For software-change validation reuse, place captures beneath the run's artifact root.

```sh
loop-engine capture-command --working-directory "$WORKING_DIRECTORY" --output-dir "$CAPTURE_DIR" --timeout-ms 60000 -- python3 -c 'print("capture smoke")'
loop-engine capture-matrix --matrix "$MATRIX" --working-directory "$WORKING_DIRECTORY" --output-dir "$CAPTURE_DIR"
# Resume the same matrix/cwd/settings only after inspecting failure and cleanup:
loop-engine capture-matrix --matrix "$MATRIX" --working-directory "$WORKING_DIRECTORY" --output-dir "$CAPTURE_DIR" --resume
# Request cancellation only for this owned capture:
loop-engine capture-abort --output-dir "$CAPTURE_DIR"
```

The single-command and matrix examples are alternatives: use a fresh output directory for each new execution. Put capture commands first, without workflow-global `--json`; they stream raw child stdout/stderr. Read `index.json` for selected immutable receipts and `state.json` for execution/cleanup state. Exit 0 alone is insufficient: inspect the selected receipts, stream integrity, timeout/abort/spawn errors and terminal cleanup.

A matrix is `{"rows":[{"id":"check","argv":["python3","scripts/check.py"],"timeout_ms":60000,"obligations":["public outcome"]}]}`. Use a matrix to preserve proof IDs/obligations; `capture-command` uses ID `command`. Declare non-secret settings through row `environment` and `inherit_environment` (single-command: repeat `--inherit-environment NAME`); never capture secrets. Read repository `docs/operational-ux-contracts.md`, **Capture**, and `docs/operational-ux-capture.schema.json` before constructing a proof matrix or adapting its index to another report format.

Resume verifies the successful contiguous prefix, exact settings, streams, current repository identity and cleanup before spawning remaining work. A failed row stops admission; prior attempts remain. Changed inputs require fresh execution, not relabeled receipts. Timeout, lost controller or pending cleanup is not cancellation proof: inspect retained attempts, `state.json` and `owned.json`, and stop if ownership/cleanup cannot be verified. Never signal remembered numeric PIDs or fabricate cleanup acknowledgments. Capture abort is separate from `cancel-invocation`; neither observer deadlines nor summaries authorize either.

## Lock work-slot policy before start

Initial bindings freeze with `initial_input`; explicit owner-attested `amend-binding` changes only future execution, never that input or previous attempts. Do not call `start` until the user has actively approved the work-slot policy for this run. Copying a provider profile is not approval. Shipped software-change, policy-document, and research profiles omit `work_slot_bindings` (or `{}`). Bound slots are opt-in: copy a skill template into the per-run profile JSON after replacing placeholders. Inspect the JSON with `preview-bindings` before that confirmation.

Ask, show the exact JSON you will freeze, and wait for explicit confirmation of all three:

1. **Whether to bind any slots**, and if so which catalog slot IDs. Sparse map: a present key is a mandatory worker for that room; an absent key stays driver-performed. `{}` or omitting the key means no bindings.
2. **Command and args per bound slot** — copy a skill template (fill `CURSOR_EXTENSION_PATH`, `CLAUDE_BRIDGE_EXTENSION_PATH`, and `MODEL` in the per-run JSON, not in the skill file), or write custom argv. Quote the exact `{command, args}` for every bound slot.
3. **Model identity per bound slot that will invoke a model-bearing CLI.** The engine freezes argv only. Encode the model in those frozen args (inner `--task-worker` JSON, repeated `--worker` JSON, or the worker CLI's own model flags). Nested inner workers count, not only the outer binding command. Do not choose a model after `start`.

Do not call `start` while any bound slot will invoke a model-bearing CLI (`pi --print`, `claude -p`, `run-plan-graph`'s inner worker, or similar) unless each model identifier is present in those frozen args, or the user has explicitly accepted that CLI's unpinned default as the model policy. An outer argv that does not name a model is not model lock-in. Do not bind a review slot when its configured policy-axis list is empty.

Provider review-binding constructors must read the same selected profile that will be passed to `start`. Software-change and research use their provider-specific per-slot `review_policies`; policy-document uses `semantic_policies`. A constructor normalizes missing required_authors to one and freezes exact assigned axes, prompts, author, model and subject metadata. Software-change allocates each policy to its first N roster authors, then groups each used author's axes in profile order; research/policy-document retain their per-policy construction. Ordinary and challenge slots stay separate. It requires pairwise-distinct non-empty author labels and non-empty models, and rejects unsupported or empty slots, missing axes/prompts/subject metadata, malformed or insufficient rosters, and invalid shipped worker-contract data.

The constructor atomically rewrites that same selected profile first. Only then may it extract and run `preview-bindings` on the resulting bindings, display the resulting profile's exact bytes and SHA-256, and obtain caller confirmation. Recheck that hash immediately before `start`, and start only that unchanged profile file. There is no post-preview merge. The frozen assignment is authoritative for a review worker; the later state instruction body is context only, not work for that reviewer.

If the user declines bindings, omit `work_slot_bindings` or set it to `{}`. A future execution mistake uses `amend-binding`; changed semantic policy or topology cannot be corrected through that path.

## Work-slot delegation

Object `initial_input` may include reserved `work_slot_bindings`: slot ID → `{command, args, context_filter?}`. A filter is the closed `{command, args}` object; it is not an arbitrary input transformer. Omit or `{}` means none. `start` rejects unknown slot IDs, unknown fields, and non-object values. `start` does not parse `fan-out` or `run-plan-graph` argv.

`show --view full` projects `work_slots` (catalog snapshot: id, state, event, optional `stdin_context_kinds`; no instruction body) and `work_slot_invocations` (including exact optional `invocation_input`, optional `assignment_selection`, per-worker `assignment_id`, and selected-attempt identity/digest/originating path when selected). Overlay status is `running` | `succeeded` | `failed` | `overrun`. Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). Completed invocations expose a provider-free `change_report` for assignment and recorded plan-task dimensions, plus run-level `assignments` and `plan_task_results`; the former `change_report.judgments` key is tombstoned and replaced by `change_report.assignments` with the same generic records. Unknown dimensions are changed. `show` remains provider-free and reads engine-owned ownership/cancellation metadata under `capture_dir`; semantic interpretation of worker output remains the driver's duty. Action/full observation arms the current visit as described above. Overlay meaning:

- succeeded: the bound CLI exited 0, not that the provider accepted the work
- failed: the bound CLI exited nonzero or the waiter vanished
- running: the waiter is alive and allowed time has not elapsed
- overrun: allowed time elapsed; wait or cancel owned work and verify cleanup, then observe before retry. Waiter loss alone is not cleanup proof.

Compare `current_state` to frozen bindings to decide the path:

- **Unbound** (cataloged or not, but absent from frozen bindings): `current_state_instructions` is the stored work body. Perform that work yourself. Then append and request the event.
- **Bound** (slot ID present in frozen bindings): instructions name the slot and its effective binding, and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`. They omit the stored work body. Do not perform that body, and do not exec the frozen command yourself. `invoke`, then use the passive monitor until completion or attention. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun wait or cancel owned work and verify cleanup, then observe before retry; on failed inspect `capture_dir/summary.json` and captured stdout before stderr.

After observing the current state with `show`, `invoke RUN_ID SLOT_ID` starts the bound worker. Optional `--input DATA_JSON` supplies one opaque JSON value to that executable; core persists and transports it without interpreting provider meaning, and it cannot be combined with assignment selection. Repeated `--assignment ID` or one `--assignments ID,...` selects only named enumerable fan-out assignments; omitted selection runs the full frozen binding. Empty, duplicate, unknown, or non-enumerable assignment selections are rejected before the waiter starts and the validated selection is recorded without rewriting the frozen binding. Invoke is rejected for unknown, unbound, or overlay-`running` slots. Live owned work, including overrun and cleanup-pending cancellation, blocks retry and departure. Wait or cancel and verify cleanup, then re-read `show`. On accept, the engine allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it on the invoke result. Inspect the recorded invocation input, selection and controls rather than inferring them from current settings. Before authoring a custom bound worker, read repository `docs/agent-usage.md`, **Work-slot delegation**, for packet keys, context forwarding, standing IDs and hidden helper contracts. Do not launch hidden waiter/join helpers yourself. Pi sessions use inherited `PI_CODING_AGENT_SESSION_DIR` or default to the worker capture's `sessions/`; do not add `--session-dir` to frozen argv or switch bound Pi commands to `--mode json`.

Use the passive monitor while overlay is `running`; inspect `invocation-progress` for graph/trace diagnosis and full show for complete invocation evidence. True inner waitpid remains in sidecar traces and `summary.json`; overlay is the bound CLI process exit. A query failure does not flip overlay. Direct `dagu status` / `dagu history` are underlying implementation surfaces, not the driver path.

`event`/`evaluate` never wait on a worker. A bound checked edge requires overlay `succeeded` matching slot ID, instruction digest, and the current slot-visit subject.

For software-change runs, read the frozen `artifact_root/intent.json` `operating_context` before phase work. Preserve reviewer captures unchanged and let the driver, not a classifier or provider, append the authoritative `finding-ledger`; an `advisory-finding-proposal` never routes work or satisfies a gate. Review `full_output_schema` captures through `attempts.json` and both raw attempt directories: one same-worker retry is allowed, and exhaustion fails closed. All software-change slots declare `finding-ledger`, `review-evidence`, `evidence-applicability`, `user-steering`, and `steering-incorporation` as eligible context so provider projections can resolve stable sources without driver-copied coordinates. Fresh review evidence uses only an invocation/assignment `origin`; review reuse uses one explicit `evidence-applicability` declaration, and semantic applicability remains the driver's judgment.

Implementation-ready and validation-ready are proof gates, not report declarations. The driver runs the provider's read-only `checkpoint` command against one existing absolute repository. It does not own Git lifecycle. If validation reports a checkpoint mismatch, request the shown check-free `revise-implementation` route, regenerate implementation and validation reports/checkpoints and fresh review evidence, then retry; do not silently substitute validation proof for implementation proof.

## Execution recovery minimum

Read the full execution correction/cancellation and override sections in repository `docs/agent-usage.md` before use. `show.result.state_visit` is the current nonnegative control revision, not a guessed integer. `show.result.effective_bindings` gives future settings; initial_input retains originals.

- `amend-binding RUN SLOT JSON` accepts exactly `{state_visit,owner,reason,binding}` with nonempty owner/reason. Replacement binding is `{command,args,context_filter?}`. It may correct future work while an older attempt runs, but never topology, schemas, policy or past commissions. Stale/unknown/terminal requests refuse.
- `invoke --preview` accepts the launch's input/selection/controls and starts no primary worker or invocation/capture. `--controls '{"max_active":1,"force_fresh":true}'` is ordinary invocation data, not an amendment. Unsupported runner controls refuse. Force-fresh prevents implicit standing/carry; selected tasks with unselected prerequisites must use full execution or a normal standing-aware subset. It does not erase arbitrary CLI-private sessions.
- `cancel-invocation RUN INVOCATION` acts on current recorded local ownership, never arbitrary PIDs. It stops and verifies disappearance/reaping before terminal invocation failure; the workflow stays active and captures survive. A durable admission marker blocks later tasks/summarizer. Each acquisition/resumption has **ten seconds**, at most **three seconds** graceful; escalation and verification share the remaining clock. Waiter loss does not prevent recorded-ownership cleanup. Interrupted controllers remain incomplete: already-running work may persist during unbounded operator delay while later admission stays blocked. Repeating the same command resumes with a new recorded ten-second attempt; it never relabels the earlier incomplete/timeout attempt as successful. Cleanup failure keeps the barrier. Wrong-run/nonrunning targets refuse unless resuming an outstanding cancellation; historical missing ownership is unsupported.
- `event RUN EVENT --override JSON` accepts exactly `{state_visit,owner,reason}` for one shown available edge after observation and quiescence. History records `overridden`, not provider allow; known bound checks are skipped and provider evaluation is not performed. No missing artifact, review pass or Bookends GREEN is invented. Later edges still need proof or another exception. A downstream checkpoint may require accepted implementation-proof-history that the skipped edge never created: missing history remains missing, so normal downstream proof can still refuse. Restore the real missing proof through supported owning-phase work, or seek a separately owner-attested exception; never synthesize historical success. Show/list/history retain has_overrides/count; final completion_mode is permanently `completed-with-overrides`. Ordinary final mode is `completed`.

These common controls do not add software-change finding/criterion/batching features to policy-document or research. New software-change contract v2 profiles (`minimal-9`, `standard-9`, `high-rigor-9`) explicitly declare independent criterion_policy. Old semantic profiles require their fixed original provider; provider-free historical reads preserve old evidence, not new capability. No active bootstrap migration is included.

## Software-change commission and durable steering

Configure each opted-in software-change binding with `"context_filter":{"command":"/absolute/software-change","args":["commission"]}` before freezing the profile. Without a filter, core forwards eligible kinds without interpreting targets. Core sends the filter `{run_id,slot_id,work_slots,artifact_root,controls,context}` and admits only a closed `{record_ids:[...]}` response that is an exact ordered subset. Unknown, duplicate, reordered or rewritten references and failed filter commands refuse before primary work. The subprocess uses the existing bounded provider transport (30 seconds). `show` remains provider-free and exposes each invocation's original `routed_inputs`; later appends do not replace them.

Inspect an unbound commission with `loop-engine --json show RUN --view full | software-change commission --slot SLOT`, adding `--task TASK` for an exact plan task (`--task summarizer` excludes task-only instructions). Inspection returns selected context, steering IDs, stale-task diagnostics and effective proof commands. The bound filter and inspection use the same provider selector. Plan-graph narrows the selected invocation snapshot into each task's `steering_context`; task targets never spread to dependants or the summarizer.

Append `user-steering` with `instruction` and one explicit target: `{"kind":"all"}`, `{"kind":"slots","ids":["design-draft","design-review"]}`, or `{"kind":"tasks","plan_revision":"1","ids":["A"]}`. Optional `supersedes:["earlier-steering-id"]` replaces whole earlier records globally, in context sequence order. A narrower replacement must reissue still-needed instructions. Unknown targets/references refuse; task-revision mismatches are visibly stale and not applied. No prose classification or live-attempt steering occurs.

Execution-only changes may add `proof_updates:[{"proof_id":"final","owner":"driver","command":"python3","args":["scripts/check.py"],"reason":"same proof, corrected executor"}]`. Name an existing plan `proof_commands` ID and an owner change and/or command with args. Selection preserves that proof's obligation and never writes or revises the plan. Workers must not interpret steering as permission to change accepted outcomes, architecture, decomposition or proof obligations. An unbound driver appends `steering-incorporation` with `{"steering_ids":["ID"],"applied":"What execution changes I applied"}`. This is a retained attestation, not proof of success or permission to advance.

## Non-run-state command: fan-out

`fan-out` does not start, advance, or record a run and does not open the run database. Each invocation emits a local Dagu `type:graph` under isolated `capture_dir/dagu-home/` (bound: packet `capture_dir`; ad-hoc: `cwd/fan-out-adhoc/<unique>`). Callers never supply Dagu YAML. The facade waitpids `dagu start --quiet --dagu-home` and does not daemonize. Use the passive monitor for waiting and `invocation-progress` for targeted graph/trace inspection. Overlay remains the facade process exit. Dagu is GPLv3: invoke the operator-provided binary as a subprocess only; do not embed its Go API. Packages do not ship `dagu` (minimum 2.14.0 on PATH).

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
```

Supply one or more strict `--worker` JSON objects with `command` and string-array `args`. Optional `preamble`, legacy required-key `output_schema`, and complete JSON Schema `full_output_schema` belong only to fan-out workers; nested `--task-worker` remains `{command,args}`. Use provider constructors for reviews. Before building a custom worker contract, read repository `docs/agent-usage.md`, **Non-run-state command: fan-out**, for accepted shapes, force-fresh constraints, stdin framing and output extraction. Unknown/malformed fields and zero workers refuse.

Use `fan-out` **ad hoc** when you want parallel worker CLIs without a run: pass `--instructions FILE` and do not send an invoke packet on stdin. Without `preamble`, worker stdin is byte-identical to the instruction-file bytes. With `preamble`, stdin is the preamble bytes unchanged, one LF appended only if the preamble does not already end in LF, literal `---\n\n`, then the unchanged instruction-file bytes (no `artifact_root` JSON). On success, stdout is exactly one JSON summary object (`dagu --quiet`). Re-invoke uses a new capture directory and a new home; prior captures are not overwritten.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`. Do not call `fan-out` yourself for that slot. Bound mode rejects `--instructions` and reads the existing worker packet (`run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context`, `assignment_selection`, and `standing_assignment_ids` supplied by invoke). Fan-out validates assignment selection through the engine before launch and accepts but does not interpret generic standing IDs. Bound stdin does not dump `instruction_body`. Without `preamble`, including an `output_schema`-only worker, stdin is compact JSON with exactly absolute `artifact_root` plus one LF, and `context` when the invoke packet carried matching `stdin_context_kinds`. With `preamble`, stdin is exactly: preamble bytes unchanged; one LF only if absent; compact JSON serialized from `{"artifact_root": <absolute>}` plus `context` when forwarded; one LF; literal `---\n\n`; and no instruction body. The location object contains no `capture_dir` or duplicate run/slot identity, and adds no new invoke-packet keys or digest input.

Default bound delivery clones the selected context and omits only each record's engine-owned top-level `data.loop_engine_origin`. This applies to every bound worker, with or without preamble; there are no opt-in flags or model classifications. Concise `origin`, record IDs/order, judgments, meaningful fields, assignment and controls remain intact; nested user/provider data is not recursively stripped. Core/provider inputs and durable history remain full. Actual `stdin` captures record the delivered bytes truthfully. Independently, `fan-out-spec.json` per-worker `routed_inputs` and summary routing retain the full original selected context, including engine origins, with `capture_format: "bound-context-projection-v1"`. Software-change verification requires that full snapshot for marked captures and refuses missing/malformed snapshots; unmarked legacy captures retain stdin-based verification. Existing output/digest/assignment/attempt/applicability checks still apply. Ad-hoc instruction bytes remain unchanged, even when JSON-looking. Plan-graph keeps its separate task/findings/steering and summarizer packets; this fan-out projection does not rewrite graph inputs or the one-worker ad-hoc-repair contract.

Bound captures contain per-worker `0/`, `1/`, … stdout/stderr and ordered `summary.json`, retained even after graph failure. Omitted `--max-active` is uncapped; set it to limit worker concurrency. Inspect every inner exit and conformance result: ordinary inner nonzero can coexist with facade exit 0, while a contract miss fails the facade. For `full_output_schema`, the identical worker gets at most one shape-correction attempt containing original output and exact errors. Preserve both raw `attempts/<N>/stdout` and `stderr`; check `attempts.json` digests, errors, selected attempt and exhaustion before accepting output. Exhaustion fails closed. Neither exit 0 nor schema conformance establishes semantic validity; triage provider judgments separately. Exact summary/attempt formats are in the same fan-out reference.

Shipped profiles omit `work_slot_bindings` (or `{}`), so slots stay driver-performed until opt-in. Use the relevant provider skill's deterministic review-binding constructor; do not hand-author one generic reviewer for a multi-axis gate, and do not bind a review slot whose configured policy-axis list is empty. Provider constructors freeze each worker's `preamble` and declared output contract inline before profile preview and lock-in; software-change review workers use assignment-specific `full_output_schema` constants. Only explicit `amend-binding` corrects future execution; it does not rewrite initial input or earlier attempts.

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

`software-change run-plan-graph` is an argv command of the software-change provider binary — not an engine operation. On a bound `implement` slot, omit invoke input for the full plan. When an existing frozen task owns a correction, use `loop-engine invoke RUN_ID implement --input '{"plan_revision":"REVISION","task_roots":["TASK_ID"]}'`; the provider requires that closed shape, current plan revision, unique known roots, and same-revision standing results for every prerequisite outside the roots-plus-dependants selection. Only when a current accepted unresolved implementation finding has no honest frozen task owner, record empty `task_ids` and invoke that unchanged slot with exact `--input '{"repair_finding_ids":["FINDING_ID"]}'`. The provider requires unique current forwarded ledger entries that are implementation-owned, accepted, unresolved, current for the verified implementation checkpoint, and no-task-routed. Malformed, empty, unknown, stale, wrong-owner/status/disposition, or task-routed repair requests refuse before Dagu resolution or mutation. Frozen `--task`/`--tasks` cannot be combined with packet input.

A valid repair executes one generic `ad-hoc-repair` assignment with the frozen task worker and checkout, no plan task or summarizer, and no `plan-task-results.json` change. Its closed stdin includes exact selected finding objects, frozen plan revision, provider-derived pre-report and repository-state identity, and the obligation to make only the correction and write a fresh report. The provider requires a schema-valid report linked to the plan whose revision is unused by the pre-proof and every accepted implementation-proof-history entry before creating a new checkpoint. Inspect `summary.json` for the generic worker/output/routed-input fields and `repair` pre/post metadata. Worker/report failure creates no post checkpoint but may leave partial checkout edits; restore or deliberately incorporate them before retry. After success, append the resolved ledger snapshot and reconfirm affected independent implementation review and validation. Process exit remains mechanical, not semantic acceptance. If an existing task owns the finding, use task selection; if decomposition is materially wrong, revise the plan. There is no direct unbound repair flag.

Direct callers may still use repeated `--task ID` (or one `--tasks ID,ID,...`); omitted selection remains full execution. `--working-directory ABSOLUTE_EXISTING_DIRECTORY` is required and must be one driver-selected existing absolute directory frozen before `start`; it is applied to every selected plan task and the summarizer or to the repair worker. Invalid or omitted values are rejected before any worker launches. Successful execution requires the selected directory to be a Git working tree for checkpoint generation; the provider does not create, manage, select, or suggest worktrees. Bound mode honors `packet.capture_dir` (per-task or `ad-hoc-repair/` output plus `summary.json`). Each invocation emits a local Dagu `type:graph` under isolated `--dagu-home` at `capture_dir/dagu-home/`. Omitted `--max-active` is `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. In full/selected mode the mandatory `summarizer` runs only after all selected tasks succeed; task failure leaves mechanical `summary.json` and captures and writes no `implementation-report.json`. The summarizer is the sole report writer in those modes. Ordinary task stdin is compact `{"artifact_root"}` JSON plus that task's plan object only, with provider-added `finding_context` containing only exact-task current accepted unresolved implementation-owned ledger entries when ledger context is present. Proposals, stale/resolved/rejected/advisory, and unrelated entries are absent. Monitor execution; use `invocation-progress` for targeted graph/trace inspection. Overlay remains the facade process exit. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths.

## Non-run-state command: preview-bindings

`preview-bindings` does not start, advance, or record a run and does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key.

It keeps outer bindings closed `{command,args,context_filter?}` and filters/nested `--task-worker` closed `{command,args}`, expands extended nested fan-out workers, reports `has_preamble`, legacy `output_schema.required`, and `full_output_schema`, and redacts preamble text from the printed binding argv. It lists detected `--model` values and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. It reports a `dagu` PATH check (minimum 2.14.0): ok with resolved path and version, or a warning naming the path or that PATH lookup found nothing. Warnings alone exit 0; `fan-out` and `software-change run-plan-graph` execute fail-close on the same condition before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). `dagu` is operator-provided and is not shipped in loop-engine or software-change packages. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## Canonical loop

Repeat until `show` reports `final` or `terminated`:

1. Read action `show`: current state/instructions, available events, blockers and work locators. Use full for frozen configuration, complete context, evaluation history, invocation/change reports or helper input.
2. If bound, `invoke` the named slot and monitor until completion or attention. Inspect retained summary/output and re-read action/full show. On overrun, wait or cancel owned work and verify cleanup before retry; on failure, inspect `summary.json` and stdout before stderr. If unbound, perform the instructed work externally, using common capture for proof commands.
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

These envelopes apply to workflow operations, not monitor JSONL or raw capture streams. Parse JSON even on nonzero exit. Treat only `completed` as operation success, not terminal workflow completion or semantic approval. Never infer state advancement from `rejected` or `error` — re-run `show` against the same database.

## Rules

- One logical mutating actor per run: serialize `append`, `event`, `invoke`, and `terminate` calls; never race them from parallel workers. Concurrent reads are fine. Context appended during an in-flight checked evaluation does not invalidate or reach that evaluation.
- When changing engine/provider code or skills, follow repository `AGENTS.md` for builds, constructor self-tests and public-boundary journeys. Synthetic evidence proves deterministic mechanics, not semantic verdict quality.
- `initial_input` is immutable run configuration; never attempt to replace it. Frozen `work_slot_bindings` are part of that input. Do not `start` until the user has approved the bindings JSON, including default-vs-custom argv and models encoded in those args — or an explicit unpinned-default acceptance for any model-bearing CLI that has no model in argv.
- Context records are immutable and append-only.
- Provider association, workflow topology, and state instructions are snapshotted at `start`; changing TOML cannot redirect an existing run.
- `show` is provider-free — it never spawns the provider. A fresh agent resumes with only the run ID, the same database, and the external references named in initial input/context/instructions.
- Final and terminated runs are read-only: `append`, `event`, and `terminate` are rejected there.
- `history` audits creation, appends, transitions, checked-transition denials, work-slot invocation started/status-changed actions, and termination — not every read, provider failure, overlay `overrun`, or other rejection; history is not provider context.
