# Loop Engine v2 — Product Requirements Document

**Status:** Living
**Target:** v0.1
**Compatibility:** Clean-slate successor; no v1 compatibility requirement
**Amended:** 2026-08-22

## 1. Product

Loop Engine is a small, durable workflow coordination system for work performed by humans, AI agents, scripts, or external systems.

It owns a workflow run's durable control state, exposes the work currently available to a caller, validates requested progress against workflow-defined policy, and preserves the meaningful semantic history of the run.

Loop Engine does not perform the primary work.

> **The engine owns workflow progress. The caller owns execution.**

The normal interaction is:

```text
inspect current work
→ perform work externally
→ append durable context as needed
→ request an event
→ engine accepts or rejects progress
→ repeat
```

The same workflow must be usable from a local agent harness, a later fresh session, a human CLI, a script, or a future cloud executor without moving workflow authority into those environments.

## 2. Problem

Agentic workflows often keep workflow state implicitly inside an agent conversation or harness: the objective, current stage, decisions, produced work, user steering, prior review findings, remaining work, and whether completion criteria have actually been satisfied.

This makes workflow correctness depend on an agent correctly remembering and following the process. It also makes durable handoff difficult and encourages workflow-specific integrations for each agent harness.

Traditional workflow engines commonly solve a broader problem involving job execution, workers, queues, timers, retries, and distributed orchestration.

Loop Engine addresses the narrower problem:

> **Persist and mechanically enforce the semantic progression of externally performed work without owning its execution.**

## 3. Scope

### 3.1 Goals

Loop Engine v2 must:

- preserve workflow control state across process, session, and actor boundaries;
- mechanically enforce permitted transitions rather than trusting callers to follow the workflow;
- remain neutral to actor type, agent harness, model, and workflow domain;
- expose enough current context through one primary read for a fresh actor to resume work;
- permit callers to add durable workflow context as work progresses;
- allow workflow-specific code to validate progress without granting it state authority;
- preserve prior validation results so iterative evaluators may account for previous findings;
- run locally without a daemon or external infrastructure;
- preserve clean seams for future cloud execution and richer case semantics;
- remain deliberately small and understandable.

### 3.2 Non-goals for v0.1

v0.1 does not provide:

- agent or LLM execution;
- background workers, scheduling, or timers;
- automatic workflow retries (bounded same-worker output conformance is a work-slot contract, not workflow progression);
- claims, leases, heartbeats, or execution attempts;
- distributed execution;
- multi-user collaboration or authorization;
- parallel or hierarchical workflow states;
- child workflows or compensation;
- workflow migration;
- provider registry or package-management APIs;
- a general expression language;
- first-class artifact, decision, approval, directive, or dependency models;
- mutable workflow variables;
- deterministic replay or event-sourced current state;
- provider compatibility-report APIs;
- a separate dynamic-guidance subsystem;
- audit import/export;
- special handling for sensitive data.

Workflow inputs and context are assumed to be reasonably sized for ordinary agentic work. Users are responsible for data placed in Loop Engine and for understanding the persistence paths used by their installation.

## 4. Core Model

Loop Engine has five primary durable concepts:

```text
Workflow
Run
State / Transition
Context Record
History Entry
```

A workflow-specific **provider** supplies the workflow definition and validation policy.

### 4.1 Workflow

A workflow defines the permitted lifecycle of a kind of work.

It contains:

```text
workflow ID
initial state
states
transitions
```

A state contains:

```text
ID
title
instructions
final flag
```

A transition contains:

```text
source state
event ID
target state
checked or check-free
```

A workflow may also declare **work slots**: provider-named jobs attached to checked edges. The catalog snapshot consumed by the engine is:

```text
slot ID
state
event
optional stdin_context_kinds
```

Catalog entries do not include instruction bodies. Omitted or empty `stdin_context_kinds` means invoke stdin has no extra context. A nonempty list causes invoke to forward stored context records whose kind is in that list, historical included, append order, unmodified. The engine does not interpret kinds or payloads. A slot ID is unique within a workflow and names an existing state plus a checked event from that state. Unbound slots (cataloged but absent from frozen `work_slot_bindings`) do not change progression rules.

For a given state, an event ID identifies at most one transition.

Workflow validation establishes **structural interpretability**, not workflow quality. A valid workflow definition has unique state IDs; its initial state names a defined state; every transition source and target names a defined state; each source-state/event pair selects at most one transition; and final states have no outgoing transitions. Cycles, unreachable states, non-final sink states, and workflows with no final state are permitted. v0.1 does not perform reachability, eventual-termination, graph-quality, or workflow-specific input/admission analysis as part of workflow-definition validation.

Cycles and revision paths are normal workflow topology.

A **check-free** transition declares that its stored graph edge is sufficient authorization to progress. It does not invoke provider evaluation.

A **checked** transition requires provider `allow` before it may commit.

Final states must have no outgoing transitions. A run whose initial state is final begins in the `final` lifecycle. Terminal runs expose no requestable events.

### 4.2 Run

A run is one durable instance of a workflow.

It contains the equivalent of:

```text
run ID
optional label
workflow snapshot
provider association
immutable initial input
current state
lifecycle
```

Lifecycle is limited to:

```text
active
final
terminated
```

Entering a final workflow state makes the run final.

An active run may be explicitly terminated.

Final and terminated runs are read-only in v0.1.

The workflow snapshot is immutable for the life of the run.

### 4.3 Context Record

A context record is immutable information deliberately added to a run so future actors and provider evaluation can use it.

It contains:

```text
record ID
kind
JSON data
creation order/time
```

Examples of workflow-defined kinds include:

```text
user-steering
artifact
decision
observation
review
external-reference
```

Core does not interpret these kinds.

Example:

```json
{
  "kind": "user-steering",
  "data": {
    "text": "The old mobile client must remain compatible."
  }
}
```

Another example:

```json
{
  "kind": "artifact",
  "data": {
    "ref": "file:///workspace/design.md",
    "revision": "3"
  }
}
```

Context records are returned to callers and providers in stable durable append order.

A context record is a caller-supplied assertion. Core guarantees durability, immutability, and ordering, but assigns no provenance, truth, approval authority, or supersession semantics to its contents.

Every checked evaluation receives the immutable initial input and all accumulated context records. v0.1 has no record-selection, filtering, mutation, or deletion semantics.

A workflow/provider may define conventions for context records that represent externally produced evidence, such as review results. Those conventions remain workflow-specific: Loop Engine core still treats the records as opaque caller-supplied assertions and does not assign them review, approval, provenance, or policy semantics.

### 4.4 Run History

Run history is an append-only, durably ordered semantic history of meaningful workflow actions. It is not an exhaustive execution or audit trace.

Exactly one aggregate history entry is created for each successful semantic action of these forms:

```text
run creation
context-record append
defined transition committed
defined checked transition denied
explicit termination
invocation started {invocation_id}
invocation status changed {invocation_id, status}
```

Invocation status in history is waiter-written `succeeded` or `failed` only. Overlay `overrun` is a reader projection on `show` (`work_slot_invocations.status`) and is not a history action. `invoke` rejections (unknown slot, unbound slot, already-running) and waiter/worker spawn failures remain operation results, not semantic history.

A transition history entry exposes the relevant:

```text
event
source state
target state
checked/check-free
outcome: committed | denied
evaluation feedback when denied
ordering/time
```

Run history does **not** record:

```text
reads
unknown or unavailable event requests
provider timeout/crash/protocol failures
unsupported evaluations
persistence failures
concurrency/staleness failures
CLI or invocation failures
```

Those are operation results or diagnostics, not semantic developments in the work.

Run history:

- explains meaningful workflow progression;
- is not replayed to derive current state;
- is not automatically treated as workflow context;
- is not supplied wholesale to provider evaluation.

Prior evaluation lineage is projected from durable checked-transition history.

### 4.5 Provider

A provider is workflow-specific code reachable through the configured provider integration.

It:

- describes a workflow;
- evaluates engine-selected checked transitions.

It does not:

- own or directly set current state;
- perform the workflow's primary work;
- select a target state;
- modify Loop Engine persistence;
- require retained in-memory state from previous provider invocations.

## 5. Authority and Workflow Semantics

The following are product invariants:

1. **The engine owns current state.** Callers and providers cannot directly assign it.
2. **Callers request events, not states.**
3. **The engine resolves transitions from the run's stored workflow snapshot.**
4. **Providers validate; they do not route.** They may allow, deny, or be unable to evaluate the selected checked transition.
5. **Each run retains its creation-time workflow topology and state instructions.**
6. **A check-free transition is authorized by its stored graph edge alone.**
7. **Rejected or errored event requests leave current state unchanged.**
8. **A committed transition and its required history entry are atomic.**
9. **A durable checked-transition denial and its feedback are recorded atomically.**
10. **Current state is authoritative; history replay is not required.**
11. **Primary workflow work remains external to Loop Engine.**
12. **Actor type and harness do not change workflow authority.**
13. **v0.1 assumes one logical mutating actor per run.** Accidental concurrent processes must not corrupt or commit conflicting workflow state.

## 6. User-Facing Operations

The semantic v0.1 surface contains eight primary operations. Exact CLI spelling remains evolvable during v0.x.

| Operation | Purpose |
|---|---|
| `start` | Create a run from a provider and initial input |
| `list` | Discover runs |
| `show` | Obtain the current working context |
| `append` | Add a context record |
| `event` | Request workflow progress |
| `history` | Inspect semantic run history |
| `terminate` | Explicitly close an active run |
| `invoke` | Start a bound work-slot worker |

### 6.1 Start

Conceptually:

```text
start <provider> <initial-input> [label]
```

The engine:

1. resolves the provider selected for the run;
2. obtains its current workflow description;
3. validates the workflow definition;
4. creates the run with the provider association, workflow snapshot, immutable initial input, and initial state;
5. durably records run creation atomically with the run.

There is no creation/admission check in v0.1.

Incomplete, ambiguous, or improvable work belongs in the workflow's initial state rather than a special pre-run evaluation phase.

If the initial state is final, the run is created final.

When object initial input contains `work_slot_bindings`, the engine freezes a map from slot ID to `{command, args}`. Omitted key and `{}` both mean no bindings. Start rejects an unknown slot ID (not in the provider `describe` catalog snapshot for this workflow), unknown fields on a binding object, and non-object values. Start does not parse `fan-out` or `run-plan-graph` argv. If the initial state is a work slot, the engine mints a **slot-visit** subject for that visit via set-current-subject.

### 6.2 Show

`show` is the primary resumption and actor interface.

A single call must expose enough information to continue normal work, including:

```text
run ID / label
workflow ID
lifecycle
current state
state title and instructions
immutable initial input
all context records in durable append order
requestable events
each event's target and whether it is checked
latest durable evaluation for each checked transition that has been evaluated
work_slots (catalog snapshot: id, state, event, optional stdin_context_kinds; no instruction body)
work_slot_invocations (invocation_id, slot_id, binding snapshot, optional assignment_selection, instruction_digest, subject, overlay status, overlay_meaning, elapsed_ms, remaining_allowed_ms, capture_dir, inner_workers, started_at, allowed_time_ms, optional exit_code, optional completed_at)
```

`work_slot_invocations.status` is the reader overlay result `running` | `succeeded` | `failed` | `overrun`, not a raw waiter-written row when overlay applies. `waiter_pid` is internal and is not in `show`. Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). Completed invocations additionally project a durable provider-free change report: subject revision, assignment and binding, run identity, declared output contract, and routed inputs for assignments; and task definition/packet, dependencies, routed inputs, worker binding, and the task-recorded repository effect for plan-task results. The run-level `change_report` exposes `assignments` and recorded `plan_task_results`. The former `change_report.judgments` show field is tombstoned: `assignments` contains the same generic assignment records under the renamed public key; provider reviewer judgments remain provider content, not this schema. Unknown report inputs are changed. `show` does not spawn a provider and does not read capture files. Overlay meaning: succeeded means the bound CLI exited 0, not that the provider accepted the work; failed means the bound CLI exited nonzero or the waiter vanished; running means the waiter is alive and allowed time has not elapsed; overrun means allowed time elapsed while the waiter is alive; run `show` immediately before re-invoking the same slot. When the current state is a bound slot, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun run `show` immediately before re-invoking the same slot; on failed inspect `capture_dir/summary.json` and captured stdout before stderr. Do not redact to only the invoke CLI. Unbound current states keep the stored instruction body.

The projection is the **chronologically latest durable evaluation** for each exact checked transition. An `allow` supersedes any earlier `deny`, and a later `deny` likewise supersedes an earlier `allow`. When the latest result is `deny`, its actionable feedback is exposed.

The projection is scoped to exact checked transitions, not merely currently requestable events. This preserves useful review feedback across revision edges without turning evaluation results into context records.

`show` does not invoke the provider.

`show` of the current state and its instructions is also the observation that arms the current state visit for mutation. The driver must observe before `append`, `event`, `invoke`, or `terminate`; `list`, `history`, and `invocation-progress` do not arm it. A state transition, including a self-loop, ends that observation, so the next mutation requires another `show`. A current observation may arm multiple mutations in the same visit.

A fresh actor **with access to any externally referenced work** must normally be able to resume from `show` without the previous session or raw history. Any workflow-specific external location or identity needed for handoff must therefore be carried in initial input, context records, or state instructions rather than ambient prior-session state.

### 6.3 Append

Conceptually:

```text
append <run> <kind> <JSON data>
```

Appending:

- requires a current `show` observation;
- creates one immutable context record;
- creates one semantic history entry;
- preserves stable append ordering;
- does not change workflow state;
- does not invoke the provider;
- cannot create or alter engine-authored invocation records.

The existing append path also exposes two explicit driver acts, `unchanged-carry` and `override-carry`. Both consult the durable change report; unchanged-carry refuses changed covered inputs, while override-carry requires the driver to name every changed input it overrides. A carry preserves originating author and selected-output identity while recording the attesting driver and act separately. The engine does not decide whether the carry was warranted.

Only active runs accept context records.

`append` against a terminal run is rejected and creates no semantic history.

### 6.4 Event

Conceptually:

```text
event <run> <event>
```

For a syntactically valid request against an existing run, absence of a matching transition from the current state is `rejected`, regardless of whether the event ID appears elsewhere in the workflow. Every event request requires a current `show` observation; an unobserved request is rejected before provider evaluation and creates no semantic history.

For a matching transition, the engine:

1. loads the authoritative active run;
2. resolves the exact transition from the stored workflow;
3. if the edge is a **bound** work slot (present in frozen `work_slot_bindings`), requires an overlay-`succeeded` invocation matching slot ID, `instruction_digest` (SHA-256 of the stored instruction body UTF-8 bytes, lowercase hex), and the current **slot-visit** subject from get-current-subject; overlay `running`, `failed`, or `overrun` do not allow the edge; this gate runs before provider `evaluate` and never waits; check-free edges are ungated;
4. for a check-free transition, atomically commits the target state and one aggregate history entry without invoking the provider;
5. for a checked transition, constructs the evaluation request and asks the provider to evaluate that exact transition;
6. after the provider returns `allow` or `deny`, verifies that the run is still active and remains in the source state against which evaluation began;
7. if that state/lifecycle check fails, treats the evaluation as stale, returns an error, and records no semantic history or evaluation lineage from the stale result;
8. on a non-stale `allow`, atomically commits the target state and one aggregate history entry, and mints a new slot-visit subject via set-current-subject when the target is a work slot (replace);
9. on a non-stale `deny`, preserves state and atomically records one aggregate denied-transition history entry containing the feedback;
10. on `unsupported` or operational failure, preserves state and returns an error without adding semantic run history.

v0.1 does **not** invalidate an in-flight evaluation merely because context records or evaluation history changed concurrently. The supported usage model assumes one logical mutating actor.

### 6.5 History

`history` returns the ordered semantic history of the run.

It exists for understanding progression, prior review decisions, context additions, termination, and work-slot invocation started/status-changed actions. Overlay `overrun` is not a history action.

It is not required for normal continuation and is not an operational trace.

### 6.6 List

`list` exposes enough information to identify resumable work:

```text
run ID
label
workflow ID
lifecycle
current state
provider identity
durable artifact location when the run has one
```

### 6.7 Terminate

`terminate` requires a current `show` observation, then closes an active run without provider evaluation and records one termination history entry. An unobserved termination is rejected without changing lifecycle or semantic history.

Final and terminated runs cannot be terminated again.

A terminal `terminate` request is rejected and creates no semantic history.

A terminated run cannot reopen in v0.1.

### 6.8 Invoke

Conceptually:

```text
loop-engine [--database DB] [--json] [--timeout-ms MS] invoke RUN_ID SLOT_ID [--assignment ID ... | --assignments ID,...]
```

`invoke` requires a current `show` observation before it starts bound work-slot work. It is rejected for an unknown slot ID, an unbound slot (no frozen `work_slot_bindings` entry), or an overlay-`running` invocation (live waiter). Overlay `overrun`, `failed`, and `succeeded` are not already-running; overlay-`overrun` is terminal for retry and a later `invoke` of the same slot is accepted. The driver must run `show` immediately before re-invoking an overrun slot so a waiter that completed after the earlier observation does not cause redundant work.

On accept, the engine snapshots the frozen `{command, args}` binding, the stored instruction body, `instruction_digest`, and the current slot-visit subject (get-current-subject only; `invoke` does not mint). It allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it on the invocation record, and returns it on the invoke result. Empty `artifact_root` is an error before spawn. It creates a running engine-authored invocation record. `append` cannot write that table. `allowed_time_ms` equals the invoke/provider timeout (`--timeout-ms`, or the same timeout the provider path already uses). The waiter is the only writer of terminal `succeeded`/`failed` plus `exit_code`.

The same `loop-engine` binary then starts hidden `wait-invocation` as a child `invoke` does not waitpid. That waiter is parent of the bound worker: it spawns `{command, args}`, waitpids the worker, writes terminal `succeeded` or `failed` plus `exit_code`, then exits. After waitpid, if `capture_dir/summary.json` is well-formed, the waiter also stores inner `command`/`args`/`exit_code` on the invocation; overlay remains the bound CLI process exit (0 → succeeded). Missing or malformed summary stores empty `inner_workers` and does not change overlay. The waiter is not a daemon and not a sibling of the worker. A vanished waiter with no terminal status is overlay-`failed`.

Hidden `stdin-exec` is a second non-user helper on the same binary. It opens `--stdin-file`, attaches that file to the child stdin, and runs `COMMAND [ARG]...` taken literally after `--` with no shell:

```text
loop-engine stdin-exec --stdin-file ABS --exit-mode sidecar|propagate [--sidecar-file ABS] -- COMMAND [ARG]...
```

Duty assignment bytes live only in that file; they are not copied onto argv or into the child environment. Sidecar mode writes the JSON object `{"exit_code": <inner waitpid as i32>}` to `--sidecar-file` after the child terminates (creating parent directories), then the helper exits 0, so a later Dagu worker step can complete without `continue_on`. Propagate mode uses the inner waitpid as the helper exit and rejects `--sidecar-file`. Spawn failure (missing binary, not executable) exits nonzero and does not write a successful sidecar. `--help` omits `stdin-exec` the same way it omits `wait-invocation`. When `PI_CODING_AGENT_SESSION_DIR` is unset in the inherited environment, stdin-exec creates `<worker-capture-dir>/sessions` and sets that variable on the child only. Frozen worker argv is not rewritten and gains no `--session-dir`. Bound Pi commands are not switched to `--mode json`.

`software-change` duplicates that helper with the same argv inside the provider crate. Plan-graph uses `--exit-mode propagate` only so the helper exit is the inner waitpid; `--sidecar-file` is rejected in that mode. `software-change --help` and `--version` omit it.

The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context` when the bound slot declared nonempty `stdin_context_kinds`, optional `assignment_selection` for a validated assignment subset, and optional provider-free `standing_assignment_ids` whenever context is forwarded. The packet is not passed on argv, environment, or a temp file. Waiter stdin is not the worker packet. Binding `{command, args}` remains the worker argv.

Later `show`, `history`, `event`, and `invoke` read stored records. They may observe waiter liveness as `running` and apply recorded `allowed_time_ms` as `overrun`. They do not waitpid the original worker.

### Other command: `invocation-progress`

`invocation-progress` is listed with `fan-out` and `preview-bindings` under Other commands, not as a ninth primary. Unlike those two, it opens the catalog. It does not append, invoke, request events, or write overlay. A failure or timeout of this query returns an error envelope and does not flip overlay. `--timeout-ms` bounds helper spawns only, never invocation `allowed_time_ms`.

```text
loop-engine [--database DB] [--json] [--timeout-ms MILLISECONDS] invocation-progress RUN_ID [INVOCATION_ID]
```

While overlay is `running`, the canonical driver poll is `show` (overlay, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, `inner_workers` empty) plus `invocation-progress` (`invocation_id`, `capture_dir`, per-step `not_started`|`running`|`reaped`, named sidecar/session traces). Graph state is Dagu helper liveness; `reaped` means the Dagu step helper finished, not overlay success and not inner waitpid 0. True inner waitpid remains in named sidecar traces and later `summary.json`; overlay remains the bound CLI process exit. `dagu status` / `dagu history` against the locator remain the underlying surface `invocation-progress` uses; they are not the driver-facing path. Session traces live under a worker directory's `sessions/` subdirectory when hidden stdin-exec set `PI_CODING_AGENT_SESSION_DIR` there; frozen argv does not add `--session-dir`. Bound Pi commands are not switched to `--mode json`.

When `INVOCATION_ID` is omitted, the unique overlay-running invocation is selected if one exists; otherwise the latest invocation by `started_at`. An early poll before the facade writes the locator can return `capture_dir` with graph omitted; retry while `show` still reports overlay `running`.

### Non-run-state CLI: `fan-out`

The `loop-engine` binary also exposes `fan-out`. It is **not** a ninth primary operation: it does not start, advance, or record a run, and it does not open the run database. `--help` lists it with `invocation-progress` and `preview-bindings`, beside and distinct from the eight run-state operations. Callers do not supply Dagu YAML. Each invocation emits a local `type:graph` under isolated `capture_dir/dagu-home/`, waitpids `dagu start --quiet --dagu-home`, and records `capture_dir/dagu-locator.json` (`dagu_home`, `dag_name`, `run_name`). While overlay is `running`, drivers poll `show` for overlay and `invocation-progress` for inner graph/traces. `dagu status` / `dagu history` against the locator remain the underlying surface `invocation-progress` uses; they are not the driver-facing path. Overlay remains the facade process exit. Dagu is GPLv3: facades invoke the operator-provided binary as a subprocess only and do not embed its Go API; packages do not ship `dagu`.

```text
loop-engine fan-out [--worker JSON]... [--instructions FILE] [--max-active N]
```

Workers come only from repeated `--worker` JSON objects. The nested object is strict: required `command` (string) and `args` (array of strings), plus optional `preamble` (string), legacy `output_schema` whose complete supported shape is `{"required":["key", ...]}`, and additive `full_output_schema` containing a complete JSON Schema (or the explicit `{schema, retry_limit: 1}` wrapper). The legacy contract remains required-key presence only; the full contract's retry limit is fixed at one. Unknown or malformed fields are rejected. This nested type does not change the outer work-slot binding type, which remains exactly `{command,args}`. Zero `--worker` entries fail closed.

Bound mode reads the existing invoke packet (including `capture_dir`, plus optional `context`, `assignment_selection`, and provider-free `standing_assignment_ids` supplied by invoke) and rejects `--instructions`. Fan-out enforces assignment selection and accepts but does not interpret standing IDs. Bound stdin does not dump `instruction_body`. A worker without `preamble` receives compact JSON with exactly absolute `artifact_root` plus one LF, and `context` when the invoke packet carried it, including when it declares only `output_schema`. A worker with `preamble` receives, in exact order: the decoded preamble bytes unchanged; exactly one appended LF only when those bytes do not already end in LF; compact JSON serialized from `artifact_root` plus `context` when forwarded; one LF; literal `---\n\n`; and no instruction body. The location object contains no `capture_dir` or duplicate run/slot identity. Digest input and gate matching do not change. Ad hoc mode has no invoke packet: with `preamble` it emits the preamble, the same conditional LF, literal `---\n\n`, then unchanged instruction-file bytes, with no fabricated artifact-root context; without `preamble`, instruction bytes remain byte-identical. Ad-hoc success prints exactly one JSON summary object (`dagu --quiet`). Re-invoke uses a new capture directory and a new home; prior captures are not overwritten.

Bound mode honors `packet.capture_dir`: it writes per-worker stdout/stderr under `0/`, `1/`, … plus `summary.json` in argv order, `fan-out-spec.json`, and the locator. Worker steps `w<index>` start concurrently with no inter-worker depends, no `continue_on`, and no `retry_policy`. Omitted `--max-active` emits no `max_active_steps` (uncapped concurrent worker start); `--max-active N` emits `max_active_steps` N so at most N worker steps run at once. Each is `action:exec` of hidden `stdin-exec --exit-mode sidecar`, except a `full_output_schema` step, whose hidden runner owns the bounded same-worker retry. Join depends on every worker, runs hidden `fan-out-join --capture-dir ABS`, writes `summary.json`, invokes no model, and does not append `review-evidence`. If the graph stops before join, the facade still writes `summary.json` from spec and sidecars. For a worker with `output_schema`, fan-out accepts stdout only when the entire whitespace-trimmed output is a JSON object or when otherwise arbitrary prose contains exactly one fenced `json` block whose content is a JSON object. Missing, malformed, non-object, or ambiguous candidates fail conformance. Fan-out checks only presence of the declared top-level keys and does not interpret names or values. For `full_output_schema`, it validates the extracted JSON candidate against every declared JSON Schema constraint, including assignment-specific constants when the caller declares them, and retries the identical command, args, preamble, assignment, and model once. The retry keeps the original assignment and appends the unchanged first stdout plus exact validation errors, asking only for a schema-conforming reconsideration. Raw bytes for each attempt live at `<worker>/attempts/<N>/stdout` and `stderr`; `<worker>/attempts.json` records schema version `1`, numbered SHA-256 digests, exact validation errors, selected attempt, and exhaustion. On success compatibility stdout/stderr contain the selected attempt; the summary names relative `attempts.json` and `selected_attempt`, which is `null` on exhaustion. Every worker summary entry contains `command`, `args`, the true process `exit_code`, `stdout_path`, and `stderr_path`. A contracted worker additionally receives `status` (`succeeded` or `failed`) and receives `conformance_error` only on failure. Both new fields are omitted for workers without a contract, preserving the uncontracted summary shape. Any conformance failure is a facade failure, but `summary.json` is written first so the driver can inspect it and captured stdout before stderr. Ordinary inner nonzero exits are recorded in the sidecar and do not fail the facade. Overlay success is not a review pass. Exit 0 and mechanical key presence do not establish semantic deliverable validity.

The facade waitpids `dagu start` for the whole graph and on success prints a JSON summary that is not a run-state envelope and not a provider evidence schema. It encodes no harness. `--help` omits hidden `fan-out-join`.

When a work slot is frozen to `loop-engine` args that begin with `fan-out`, the legal start remains `loop-engine invoke RUN_ID SLOT_ID`; repeated `--assignment ID` or one `--assignments ID,...` may select a named enumerable subset. Omission runs every frozen assignment. Empty, duplicate, unknown, and non-enumerable selections refuse before the waiter starts, and the validated selection is recorded on the invocation while the frozen binding remains unchanged. `invoke` execs that frozen argv with the existing worker packet on stdin. Callers who want reviewers put `--worker` JSON objects in those frozen binding args at start after `preview-bindings` and lock-in; bindings cannot be patched mid-run. A usable review binding is caller-supplied `--worker` objects frozen at `start` — not a stock zero-worker `fan-out` argv.

`software-change run-plan-graph` is an argv command of the software-change provider binary, not an engine operation. Its required invocation is `software-change run-plan-graph --working-directory ABS [--task-worker JSON] [--task ID ... | --tasks ID,ID,...] [--max-active N]`; optional selection runs selected plan-task roots plus their dependants and refuses missing prerequisites before Dagu starts. ABS must be one existing absolute directory selected and maintained by the driver. Omission, relative paths, nonexistent paths, and non-directories are rejected before any Dagu graph worker starts. Bound mode honors `packet.capture_dir` (per-task `{task_id}/` plus `summary.json`). The selected directory is the graph-level cwd for every selected plan task and the summarizer; a successful implementation graph must be a Git working tree so the provider can write its implementation checkpoint, but the provider does not create, discover, select, reuse, merge, clean, or otherwise manage or suggest worktrees. Hidden `software-change stdin-exec` uses the same argv as `loop-engine stdin-exec` and is omitted from `--help`/`--version`; plan-graph uses `--exit-mode propagate` only. Each invocation emits a local Dagu `type:graph` under `capture_dir/dagu-home/` (fail-fast, no `continue_on`) and waitpids `dagu start`. Omitted `--max-active` remains `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks. While overlay is `running`, drivers poll `show` for overlay and `invocation-progress` for inner graph/traces; `dagu status` / `dagu history` remain the underlying surface, not the driver-facing path. Overlay remains the facade process exit. A mandatory `summarizer` still runs after those tasks and is the sole writer of `artifact_root/implementation-report.json`. When `--task-worker` is omitted, the default inner worker is `pi --print --no-skills --no-extensions`; it does not pass `--no-context-files` and does not pass `--tools`, so bash, edit, write, and AGENTS.md remain available. That omitted-`--task-worker` fallback does not add `-e` paths. Bound implement is opt-in; shipped software-change profiles omit `work_slot_bindings`.

### Non-run-state CLI: `preview-bindings`

The `loop-engine` binary also exposes `preview-bindings` with `fan-out` and `invocation-progress`. They are other commands, not a ninth primary. `preview-bindings` does not start, advance, or record a run, and it does not open the run database.

```text
loop-engine preview-bindings [JSON|@FILE]
```

Omitted operand reads stdin; `@FILE` reads that path; otherwise the operand is inline JSON. Accepted JSON is a `work_slot_bindings` map or an object containing that key. The outer binding remains strict `{command,args}`. It expands strict nested `--task-worker` `{command,args}` objects and extended nested fan-out `--worker` objects, reporting `has_preamble`, legacy `output_schema.required`, and `full_output_schema` without exposing preamble text. It lists detected `--model` values and warns on unpinned `pi`, PATH versus absolute command, missing `--no-skills`, `--no-extensions` without `-e`, and the 30-second invoke default. Missing `--no-extensions` is not a required warning. It also reports a `dagu` PATH check (minimum 2.14.0): ok with resolved path and version, or a warning naming the path or that PATH lookup found nothing. Warnings alone exit 0; `fan-out` and `software-change run-plan-graph` execute fail-close on the same condition before any worker spawn. Isolated Dagu home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). loop-engine and software-change release packages do not contain, vendor, or install `dagu`. It exits nonzero on malformed input and when any `fan-out` binding has zero `--worker` entries. `start` still does not parse `fan-out` argv; preview is the fail-closed check for that freeze.

## 7. Operation Outcomes

Every dispatched semantic operation has one of three outcomes.

### `completed`

The operation achieved its purpose.

### `rejected`

The request was understood, but workflow or lifecycle semantics denied it.

Examples:

```text
no transition exists for the requested event from the current state
provider denied a checked transition
mutation requested against a terminal run
invoke of an unknown, unbound, or overlay-running slot
bound checked event without overlay-succeeded invocation
```

### `error`

The operation could not be reliably evaluated or committed.

Examples:

```text
provider integration could not execute
provider response was invalid
provider returned unsupported for the stored workflow/action
durable persistence failed
workflow state changed before an evaluated transition could commit
```

Machine-readable interfaces must distinguish these three outcomes and expose actionable codes/messages where applicable.

Exact envelope field names, CLI formatting, and process exit codes belong to technical design.

## 8. Provider Interface

The semantic provider interface has exactly two operations in v0.1:

```text
describe
evaluate
```

Exact transport, serialization, process lifecycle, and framing belong to technical design.

Provider interaction must be stateless from Loop Engine's perspective: correct evaluation cannot depend on retained process memory from earlier invocations.

### 8.1 Describe

`describe` returns the provider's current workflow definition:

```text
workflow ID
initial state
states
transitions
work_slots (optional catalog: id, state, event, optional stdin_context_kinds)
```

The engine validates the definition before creating a run according to the structural-validity requirements in Section 4.1. Work-slot catalogs are snapshotted with the workflow; instruction bodies stay on states, not on catalog entries.

Start snapshots the workflow `describe` returns for that caller object. The describe envelope is `{operation: describe}` plus optional `initial_input` (the same caller JSON start freezes). Core does not interpret provider-specific keys such as `review_policies` and gains no restitch language. Providers may vary topology from that object; active runs keep the snapshotted workflow. Omitted `initial_input` is a union or otherwise input-independent catalog, as the provider defines.

The complete validated workflow definition is snapshotted into the run.

### 8.2 Evaluate

`evaluate` validates one exact engine-selected checked transition.

It receives the equivalent of:

```text
the run's stored workflow definition
immutable initial input
all context records in durable append order
the exact selected transition
prior durable allow/deny results for that exact checked transition
```

The exact transition is already identified by its stored source state and event; the target is included in the supplied action.

The raw run history is not supplied.

A provider may use workflow-specific policy configuration contained in immutable initial input together with caller-supplied evidence contained in context to decide whether the selected transition is authorized. The semantic work that produces that evidence may be performed externally by a human, agent, script, or other system; `evaluate` does not imply that the provider itself must perform that work.

### 8.3 Evaluation Lineage

Evaluation lineage is scoped to the **exact checked transition** in the stored workflow. Because a state/event pair uniquely identifies a transition, no separate check identity is required.

All durable `allow` and `deny` evaluations for that checked transition contribute to its lineage, even if the workflow leaves and later returns to that state.

Prior evaluations are supplied in stable chronological order.

A prior evaluation contains the equivalent of:

```text
evaluated transition
result: allow | deny
deny feedback, when applicable
ordering/time
```

Only semantically durable evaluations enter the lineage:

- a committed `allow` is included;
- a durably recorded `deny` is included;
- `unsupported` is not included;
- provider/process/protocol failures are not included;
- stale `allow` or `deny` results and otherwise uncommitted evaluations are not included.

The provider decides whether and how to use prior evaluation lineage for validation diagnostics or evidence aggregation. A provider may deliberately ignore previous evaluations when validating current evidence without requiring a different engine semantic; lineage never performs semantic review, which remains external.

### 8.4 Evaluation Results

The provider returns exactly one of:

```text
allow
deny
unsupported
```

`allow` contains no required feedback payload and authorizes only the exact transition supplied by the engine.

`deny` returns actionable:

```text
code
message
optional opaque details
```

That feedback becomes durable semantic history and is available to later evaluations of the same transition.

`unsupported` means the current provider implementation cannot evaluate the stored workflow/action. It is surfaced as an `error`, does not advance the run, does not enter semantic history, and does not enter evaluation lineage.

Providers cannot route to another state or create context records through `evaluate` in v0.1.

### 8.5 Provider Association and Evolution

A run remains associated with the provider selected when it was created. Later alias or configuration changes must not silently cause that run to be evaluated by a different provider.

How provider identity and association are represented is a technical-design decision.

The implementation reached through that association may evolve while a run is active.

Workflow snapshotting freezes the engine-enforced topology and state instructions for the run. It does **not** freeze provider implementation or validation behavior.

Every later evaluation receives the run's stored workflow definition. If the current provider implementation can no longer evaluate it, the provider returns `unsupported`.

v0.1 has no compatibility subsystem, workflow migration mechanism, or provider-pinning requirement.

## 9. Durable State, Ordering, and Concurrency

Loop Engine requires local durable persistence and no daemon or external infrastructure for normal local operation. The exact persistence technology and schema belong to technical design.

Durable state must preserve:

```text
runs
workflow snapshots
provider associations
immutable initial input
ordered context records
ordered semantic history
```

The following semantic mutations must be atomic:

```text
run creation + creation history
context-record append + history
check-free transition + history
allowed checked transition + evaluation result + history
denied checked transition + evaluation feedback history
termination + history
```

Current run state is authoritative.

Context-record and history ordering must be stable across process restarts and independent of wall-clock timestamp ambiguity.

When overlapping event attempts compete against the same pre-mutation run state, at most one may commit; any competing attempt made stale by the committed mutation must fail without producing a conflicting semantic effect.

A checked evaluation is stale in v0.1 when the run's state or lifecycle changes while evaluation is in flight. Staleness applies regardless of whether the provider returned `allow` or `deny`: the stale result produces no semantic history or evaluation lineage.

Concurrent context changes alone do not invalidate an evaluation in v0.1.

Loop Engine's atomicity and concurrency guarantees apply to **Loop Engine's own durable workflow state**. Provider evaluation may observe externally managed work such as repository files or documents, but v0.1 does not make that external observation atomic with the subsequent Loop Engine transition commit and does not lock or version external work on the provider's behalf.

If a semantic operation cannot durably commit the history required by its semantics, it must not report semantic success.

No caller-facing leases, revisions, idempotency keys, or retry protocol are required.

## 10. Reference Workflow A — Software Change

The primary reference workflow validates agentic software-engineering use.

Required topology (union catalog when `review_policies` is omitted; a present `review_policies` object keeps only live review states):

```text
explore
  └─ intent-ready [checked] → intent-review

intent-review
  ├─ approved [checked] → intent-adversarial-review
  └─ revise [check-free] → explore

intent-adversarial-review
  ├─ approved [checked] → design
  └─ revise [check-free] → explore

design
  └─ design-ready [checked] → design-review

design-review
  ├─ approved [checked] → design-adversarial-review
  ├─ revise [check-free] → design
  └─ revise-intent [check-free] → explore

design-adversarial-review
  ├─ approved [checked] → plan
  ├─ revise [check-free] → design
  └─ revise-intent [check-free] → explore

plan
  └─ plan-ready [checked] → plan-review

plan-review
  ├─ approved [checked] → plan-adversarial-review
  ├─ revise [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

plan-adversarial-review
  ├─ approved [checked] → implement
  ├─ revise [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

implement
  └─ implementation-ready [checked] → implementation-review

implementation-review
  ├─ approved [checked] → implementation-adversarial-review
  ├─ revise [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

implementation-adversarial-review
  ├─ approved [checked] → validation
  ├─ revise [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

validation
  └─ validation-ready [checked] → validation-review

validation-review
  ├─ approved [checked] → validation-adversarial-review
  ├─ revise [check-free] → validation
  ├─ revise-implementation [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

validation-adversarial-review
  ├─ passed [checked] → end
  ├─ revise [check-free] → validation
  ├─ revise-implementation [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

end [final]
```

Checked-edge work slots for this workflow:

```text
intent-draft (explore, intent-ready)
intent-review (intent-review, approved)
intent-adversarial-review (intent-adversarial-review, approved)
design-draft (design, design-ready)
design-review (design-review, approved)
design-adversarial-review (design-adversarial-review, approved)
plan-draft (plan, plan-ready)
plan-review (plan-review, approved)
plan-adversarial-review (plan-adversarial-review, approved)
implement (implement, implementation-ready)
implementation-review (implementation-review, approved)
implementation-adversarial-review (implementation-adversarial-review, approved)
validation-draft (validation, validation-ready)
validation-review (validation-review, approved)
validation-adversarial-review (validation-adversarial-review, passed)
```

Shipped software-change profiles omit `work_slot_bindings` (or `{}`), so draft slots stay driver-performed by convention. Review slots are bindable. Bound workers are opt-in skill templates: keep `--no-skills --no-extensions`, add `-e CURSOR_EXTENSION_PATH -e CLAUDE_BRIDGE_EXTENSION_PATH`, name `--model MODEL`, and fill those placeholders in the per-run profile JSON. A usable review binding is caller-supplied `--worker` objects frozen at `start` after `preview-bindings` and lock-in. Documented review `pi` worker examples include `--print --no-skills --no-extensions -e CURSOR_EXTENSION_PATH -e CLAUDE_BRIDGE_EXTENSION_PATH --tools read,grep,find,ls` and must not pass `--no-context-files`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning. Policy-document and research shipped profiles stay unbound.

The provider may inspect repository state, documents, tests, reviews, or other software-specific information. Core understands none of those concepts.

Review-state routing is explicit in this reference graph: external review operators select the phase owning an accepted material defect through `revise-intent`, `revise-design`, or `revise-plan`. Nearest `revise` from parent and adversarial review returns to that phase's draft. Validation-report-local defects stay in the validation draft: edit and recheck `validation-report.json`, then retry the next checked hop (`validation-ready` or `passed`). Validation-review and validation-adversarial-review also expose `revise-implementation` to implement. Candidate reviewer output is triaged before append or artifact mutation, and disputed candidates use focused external reconsideration. A late finding requires current evidence, violated in-scope obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked; prior visibility or reviewer overlook does not waive known materiality. Comprehensive-first review still bars drip-feeding, and unrelated reopening must meet independent scope/materiality burden. Quiet, progress, and thrash count per review state on the post-triage accepted-finding set; they replace a numeric breaker and never waive a known defect. These are provider/operator conventions, not Loop Engine core policy or a review subsystem.

### 10.1 Run-Configured Semantic Review Policies

The software-change provider's shipped review contract remains external and deterministic at its boundary. Binaries built from this candidate source expose identity through `--help`/`-h` and `--version`/`-V` before stdin; public v0.2.2 binaries predate those flags. Direct pushes to `main` reuse read-only production preflight at the pushed SHA. Calibration fixtures are supplied-material-only and use stable fictional path labels with shipped companions, so review does not resolve labels against a live checkout. Stale evidence is recovery context under `details.informational`; current unsatisfied obligations remain under blocking `details.diagnostics`.

At run creation, the software-change workflow may receive semantic review policies in immutable initial input. These policies configure what must be reviewed at the live review gates. A present `review_policies` object is applied at `describe`: only nonempty gate lists keep those review states, and start snapshots that live graph. Core does not interpret `review_policies`. The snapshotted topology for an existing run does not change during that run.

The initial input may contain the equivalent of:

```text
change request / objective
repository or workspace reference, when required
semantic review policies grouped by review gate
optional supporting context
```

The review gates are:

```text
intent-review
intent-adversarial-review
design-review
design-adversarial-review
plan-review
plan-adversarial-review
implementation-review
implementation-adversarial-review
validation-review
validation-adversarial-review
```

A policy is a durable semantic requirement, conceptually identified by a stable workflow-specific ID and human-readable description. The exact input schema belongs to the software-change provider rather than Loop Engine core. Different runs may therefore snapshot different live graphs from the same union phase table. Because the policies are part of immutable initial input, the configured policy set and snapshotted topology for an existing run do not silently change during that run.

Configured policies are exposed to callers through the ordinary `show` projection because `show` already returns immutable initial input. They are **not formatted prompts** and the provider interface does not gain a prompt-generation operation. A human, agent harness, or other caller may render a policy into whatever review instructions or prompts are appropriate for that reviewer.

Semantic review work is performed externally. For each configured policy axis, a human, agent, script, or other system may append a workflow-specific context record containing the review result and actionable findings. The exact review-evidence schema is a software-change-provider convention; Loop Engine core treats it as opaque context and assigns it no truth, provenance, or approval authority.

For the reference software-change provider, checked progression at a review/validation gate validates whether the durable run data contains acceptable review evidence for the policies configured for that gate. It may deny because required evidence is missing or reports failure, and its denial feedback should identify the unsatisfied policy obligations. The reference provider does not itself perform the underlying semantic review and does not generate reviewer prompts.

This is a reference-workflow convention, not a new engine-level semantic-policy, validator, review-result, or prompt abstraction.

The workflow must support:

- a minimal initial idea;
- substantial initial context representing already-discussed intent/design/planning;
- semantic review policies configured per review/validation gate at run creation;
- different policy configurations across runs, each snapping the live graph `describe` returns for that caller object;
- external human/agent review across configured policy axes;
- durable review evidence appended as ordinary context;
- user steering appended during the run;
- checked completion/review decisions;
- check-free revision/backtracking;
- repeated review/revision cycles;
- prior review evidence and evaluation lineage available to later checks;
- durable handoff to a fresh actor;
- successful finalization.

A revision/backtracking edge must remain usable from the stored graph even when the provider is unavailable.

If external repository/workspace identity is required for handoff, the workflow must carry it durably through initial input, context, or instructions rather than rely on the prior harness session.

## 11. Reference Workflow B — Policy-Conformant Document

The second reference workflow validates domain neutrality and workflows combining deterministic and semantic evaluation.

Its purpose is to **draft or audit a target document so that it satisfies deterministic and semantic policies supplied as initial input**.

Example targets include:

- a README serving as a condensed PRD and getting-started guide;
- an `AGENTS.md` explaining how any capable agent can successfully perform work in a repository;
- another document governed by explicit structural and semantic requirements.

### 11.1 Initial Input

The workflow's initial input contains the equivalent of:

```text
mode: draft | audit
target document or durable target reference
deterministic policies
semantic policies
optional supporting context
```

Loop Engine treats these values as opaque workflow input.

### 11.2 Topology

Required topology:

```text
prepare
  └─ ready [check-free] → deterministic-review

deterministic-review
  ├─ passed [checked] → semantic-review
  └─ revise [check-free] → prepare

semantic-review
  ├─ passed [checked] → end
  └─ revise [check-free] → prepare

end [final]
```

Checked-edge work slots for this workflow:

```text
deterministic-review (deterministic-review, passed)
semantic-review (semantic-review, passed)
```

There is no work slot for `prepare` → `ready`; that edge is check-free.

`prepare` instructs the actor to draft the target or revise the existing target.

`deterministic-review` validates mechanically testable policies such as:

```text
required sections exist
prohibited sections are absent
required commands or references are present
formatting or structural rules hold
links or paths resolve
```

`semantic-review` evaluates policies requiring judgment, such as whether:

```text
a README accurately condenses product intent
getting-started instructions are sufficient
an AGENTS.md gives an unfamiliar agent enough repository context
instructions are precise and non-contradictory
content is appropriately scoped and concise
```

Semantic judgment stays external to provider evaluation. A reviewer or model may produce ordinary `review-evidence` context records; provider validates strict shape, configured policy identity, target identity, profile version, and SHA-256 of exact bytes.

The final `semantic-review → end` evaluation must establish the target's **complete current conformance**, including re-establishing deterministic policies as necessary. This prevents edits made after an earlier deterministic pass from allowing finalization with newly introduced deterministic violations.

Previous durable `allow`/`deny` results for the exact semantic transition are supplied to subsequent evaluations. The provider may use them to inform validation diagnostics or evidence aggregation, or deliberately ignore them when validating current evidence. Lineage never performs semantic review; semantic judgment remains external.

Failed checks deny progression and return actionable feedback. Revision is represented by the caller taking a check-free `revise` edge and then progressing through review again.

The workflow must demonstrate:

- both draft and audit modes;
- deterministic and semantic policies supplied as initial input;
- provider-owned deterministic evaluation plus provider validation and aggregation of externally produced semantic evidence, with semantic judgment remaining external;
- repeated edit/revalidate cycles;
- deterministic conformance re-established before finalization after revisions;
- use of prior evaluation findings;
- provider-controlled workflow progression and deterministic/evidence validation while semantic review remains external;
- durable actor handoff;
- no document-specific core semantics.

## 12. Reference Workflow C — Research

The third reference workflow validates a durable research process: scope a question, gather sources, adversarially verify claims, and synthesize a cited conclusion.

Required topology:

```text
scope
  └─ scoped [checked] → gather

gather
  ├─ gathered [checked] → verify
  └─ revise [check-free] → scope

verify
  ├─ verified [checked] → synthesize
  ├─ revise [check-free] → gather
  └─ revise-brief [check-free] → scope

synthesize
  ├─ completed [checked] → end
  ├─ revise [check-free] → verify
  ├─ revise-sources [check-free] → gather
  └─ revise-brief [check-free] → scope

end [final]
```

Checked-edge work slots for this workflow:

```text
scope (scope, scoped)
gather (gather, gathered)
verify (verify, verified)
synthesize (synthesize, completed)
```

Provider evaluation validates artifact schemas and revision links for `brief.json`, `sources.json`, `verification.json`, and `report.json`, then aggregates external `review-evidence` at verify and synthesize. It does not fetch the web, invoke models, or judge semantic truth. Search, fetch, and writing stay with callers.

In the shipped standard profile, checked `verified` requires independent evidence for `claim-grounded` and `adversarial`; checked `completed` requires independent evidence for `cited-conclusion` and `scope-faithful`. Checked `scoped` and `gathered` are schema and revision-link only.

Owning-phase `revise`, `revise-brief`, and `revise-sources` edges are check-free as in the topology.

Operator procedure lives in `crates/research-provider/README.md`.

## 13. Harness and Handoff Usability

A generic agent integration should normally require only:

```text
show
append
event
invoke
```

Conceptually:

```text
context = show(run)

while context.lifecycle == active:
    if the current state is a bound work slot:
        invoke the named slot
        poll show until work_slot_invocations overlay status is succeeded,
          failed, or overrun; do not perform the stored work body
        on overlay overrun, run show immediately before re-invoking
        on failure, inspect summary.json and captured output before stderr
    else:
        perform current state's instructions
    append durable context when useful
    request an available event

    if rejected:
        use returned feedback and continue work

    context = show(run)
```

`event` and provider `evaluate` never wait on a worker. Hidden `wait-invocation` is not a harness command.

A harness may retain additional private conversational context, but correct workflow continuation must not depend on it.

No workflow-specific harness extension is required to enforce workflow progression.

## 14. Acceptance Criteria

v0.1 is complete when the following are demonstrated end to end.

### 14.1 Workflow Authority

### LE-1: A caller cannot directly set current state.
- Status: live
- Coverage: e2e/journey

### LE-2: A structurally uninterpretable workflow definition cannot create a run, while structurally valid unusual topology such as cycles, unreachable states, non-final sinks, or absence of a final state is permitted.
- Status: live
- Coverage: e2e/journey

### LE-3: Workflow topology does not vary per run based on initial input or accumulated context.
- Status: tombstone

### LE-4: A syntactically valid but unavailable event is rejected without changing state or semantic history.
- Status: live
- Coverage: e2e/journey

### LE-5: A check-free transition commits from stored graph semantics without provider evaluation.
- Status: live
- Coverage: e2e/journey

### LE-6: A checked transition cannot advance without provider `allow`.
- Status: live
- Coverage: e2e/journey

### LE-7: Provider output cannot select a different target state.
- Status: live
- Coverage: e2e/journey

### LE-8: Rejected and errored requests preserve current state.
- Status: live
- Coverage: e2e/journey

### LE-9: Accepted transitions and their required history commit atomically.
- Status: live
- Coverage: e2e/journey

### LE-10: Current state survives process restart.
- Status: live
- Coverage: e2e/journey

### LE-11: Active runs retain their stored topology and instructions after the provider's current `describe` output changes.
- Status: live
- Coverage: e2e/journey

At `start`, the engine snapshots the workflow returned by `describe` for that
caller's immutable initial input. Later provider changes may affect new runs or
make evaluation of a stored action unsupported, but they cannot replace an
active run's stored states, transitions, work-slot catalog, or instructions.

### LE-12: A provider implementation unable to evaluate a stored workflow/action fails explicitly without advancing the run.
- Status: live
- Coverage: e2e/journey

### LE-13: Final states cannot declare outgoing transitions.
- Status: live
- Coverage: e2e/journey

### LE-14: An initially-final run is created final.
- Status: live
- Coverage: e2e/journey

### LE-15: Terminal runs expose no requestable events and reject `append`, `event`, and `terminate` without semantic history.
- Status: live
- Coverage: e2e/journey

### 14.2 Durable Context and Handoff

### LE-16: Initial input is immutable after creation and survives restart.
- Status: live
- Coverage: e2e/journey

### LE-17: Context records survive restart in stable append order.
- Status: live
- Coverage: e2e/journey

### LE-18: Context records have no engine-level truth, provenance, approval, or supersession semantics.
- Status: live
- Coverage: e2e/journey

### LE-19: Every checked evaluation receives all accumulated context records in durable append order.
- Status: live
- Coverage: e2e/journey

### LE-20: `show` gives a fresh actor enough information to resume without the previous conversation or raw history, assuming access to externally referenced work.
- Status: live
- Coverage: e2e/journey

### LE-21: Workflow-specific external work identity required for handoff is durably represented through opaque workflow data or instructions rather than ambient session state.
- Status: live
- Coverage: e2e/journey

When continuation requires a repository, workspace, artifact root, document,
or other external subject, a fresh actor must be able to recover its identity
or location from initial input, context, or stored instructions. Correct
handoff cannot depend on prior chat, an unstated working directory, or
actor-private memory.

### LE-22: Appended steering is visible to subsequent actors and evaluations.
- Status: live
- Coverage: e2e/journey

### LE-23: `show` preserves review feedback across revision edges by exposing the chronologically latest durable evaluation per checked-transition lineage.
- Status: live
- Coverage: e2e/journey

### LE-24: Later durable evaluations supersede earlier ones in either direction: `allow` can supersede `deny`, and `deny` can supersede `allow`.
- Status: live
- Coverage: e2e/journey

### 14.3 Run History and Evaluation Lineage

### LE-25: Run history contains only the semantic actions defined in Section 4.4.
- Status: live
- Coverage: e2e/journey

### LE-26: Reads, unavailable events, unsupported evaluations, and operational failures do not pollute semantic history.
- Status: live
- Coverage: e2e/journey

### LE-27: One transition request creates at most one aggregate transition history entry.
- Status: live
- Coverage: e2e/journey

### LE-28: History ordering remains stable across restart.
- Status: live
- Coverage: e2e/journey

### LE-29: A committed `allow` and durably recorded `deny` survive process and actor changes.
- Status: live
- Coverage: e2e/journey

### LE-30: Evaluation lineage is scoped to the exact checked transition, not a provider policy key.
- Status: live
- Coverage: e2e/journey

The lineage key is the stored source-state/event pair, which uniquely selects
a transition. Provider policy-axis IDs may be reused across gates or several
axes may contribute to one gate; neither changes how the engine groups prior
durable `allow` and `deny` results.

### LE-31: Later evaluations receive prior durable `allow`/`deny` lineage in stable order.
- Status: live
- Coverage: e2e/journey

### LE-32: `unsupported`, provider failures, stale results, and uncommitted evaluations do not enter evaluation lineage.
- Status: live
- Coverage: e2e/journey

### LE-33: Providers may use or ignore prior evaluations without different engine semantics.
- Status: live
- Coverage: e2e/journey

### LE-34: `allow` requires no durable feedback payload; `deny` carries durable actionable feedback.
- Status: live
- Coverage: e2e/journey

### LE-35: Raw run history is not implicitly supplied as evaluation context.
- Status: live
- Coverage: e2e/journey

### 14.4 Concurrency

### LE-36: Overlapping event attempts competing against the same pre-mutation run state cannot both produce conflicting commits.
- Status: live
- Coverage: e2e/journey

### LE-37: An in-flight checked evaluation made stale by a state or lifecycle change produces no transition, semantic history, or evaluation lineage regardless of whether the provider returned `allow` or `deny`.
- Status: live
- Coverage: e2e/journey

### LE-38: Concurrent context appends alone are explicitly outside v0.1 evaluation-staleness guarantees.
- Status: live
- Coverage: e2e/journey

### 14.5 Software-Change Workflow

### LE-39: A minimal idea can be driven to completion.
- Status: live
- Coverage: e2e/journey

### LE-40: Substantial already-known intent/design/planning context can use the same workflow.
- Status: live
- Coverage: e2e/journey

### LE-41: A run can configure semantic review policies for the review/validation gates in immutable initial input.
- Status: live
- Coverage: e2e/journey

### LE-42: The configured policies are available to a fresh actor through `show` without a provider call or separate policy-discovery operation.
- Status: live
- Coverage: e2e/journey

### LE-43: The same workflow topology and software-change provider mechanism can execute runs with materially different review-policy configurations.
- Status: live
- Coverage: e2e/journey

### LE-44: Human or agent reviewers can perform each configured semantic review externally and append the resulting pass/fail findings as ordinary durable context using the software-change provider's workflow-specific evidence convention.
- Status: live
- Coverage: e2e/journey

### LE-45: The reference software-change provider does not itself perform the semantic review or generate formatted review prompts; it validates whether the configured review obligations have acceptable durable evidence before allowing progression from a review/validation gate.
- Status: live
- Coverage: e2e/journey

### LE-46: Missing or failed required review evidence denies the corresponding checked approval with actionable feedback identifying the unsatisfied policy obligation.
- Status: live
- Coverage: e2e/journey

### LE-47: Acceptable evidence for all configured policies at a gate allows that policy portion of the gate to pass without introducing engine-level policy or review-result semantics.
- Status: live
- Coverage: e2e/journey

### LE-48: At least one review denial and revision cycle is demonstrated.
- Status: live
- Coverage: e2e/journey

### LE-49: After a review denial and check-free revision edge, a fresh actor can see the configured policies, durable review evidence, and actionable review feedback through `show` without reading `history`.
- Status: live
- Coverage: e2e/journey

### LE-50: After a later successful evaluation, the previous denial is no longer projected as the latest result for that transition.
- Status: live
- Coverage: e2e/journey

### LE-51: User steering can affect later work and evaluation.
- Status: tombstone

### LE-52: Prior review evidence and evaluation lineage can inform later review/check cycles.
- Status: live
- Coverage: e2e/journey

### LE-53: A run can move between distinct actor sessions or harnesses.
- Status: live
- Coverage: e2e/journey

### LE-54: A check-free revision edge remains usable when provider evaluation is unavailable.
- Status: live
- Coverage: e2e/journey

### 14.6 Policy-Document Workflow

### LE-55: Draft and audit modes both work.
- Status: live
- Coverage: e2e/journey

### LE-56: Deterministic policy failure blocks progression with actionable findings.
- Status: live
- Coverage: e2e/journey

### LE-57: Semantic policy failure blocks progression with actionable findings.
- Status: live
- Coverage: e2e/journey

### LE-58: Successive semantic reviews may use previous findings.
- Status: live
- Coverage: e2e/journey

### LE-59: Successive external semantic reviews may use previous findings; provider may use prior lineage only to inform validation diagnostics or evidence aggregation, or ignore it when validating current evidence.
- Status: live
- Coverage: e2e/journey

### LE-60: The actor can revise and request evaluation repeatedly until policies pass.
- Status: live
- Coverage: e2e/journey

### LE-61: After deterministic review passes, a later document revision that violates a deterministic policy cannot finalize until deterministic conformance is re-established according to the external state observed by provider evaluation.
- Status: live
- Coverage: e2e/journey

### LE-62: The workflow does not rely on Loop Engine atomically locking, versioning, or committing the external document together with workflow state.
- Status: live
- Coverage: e2e/journey

### LE-63: README-like and `AGENTS.md`-like policy sets require no core changes.
- Status: live
- Coverage: e2e/journey

### 14.7 Research Workflow

### LE-64: An operator can start a research run through Loop Engine using `start` / `show` / `append` / `event`, and `invoke` when a slot is bound.
- Status: live
- Coverage: e2e/journey

### LE-65: Topology covers scope, gather, adversarial verify, and synthesize.
- Status: live
- Coverage: e2e/journey

### LE-66: Checked transitions refuse until artifacts satisfy declared structure and independent evidence satisfies declared review obligations at verify and synthesize.
- Status: live
- Coverage: e2e/journey

### LE-67: Local blackbox tests exercise at least one checked denial and a successful completion.
- Status: live
- Coverage: e2e/journey

### LE-68: CI preflight builds the research binary and runs a source journey; archive-smoke runs a packaged journey after materializing embedded data.
- Status: live
- Coverage: e2e/journey

### LE-69: cargo-dist plan and release-gate assertions include the research binary.
- Status: live
- Coverage: e2e/journey

### LE-70: The provider does not fetch, invoke models, or judge semantic truth.
- Status: live
- Coverage: e2e/journey

### 14.8 Operational Simplicity

### LE-71: Local operation requires no daemon or external infrastructure beyond Loop Engine's local durable state and configured provider integration. Hidden `wait-invocation` is a short-lived per-invocation waiter, not a background service.
- Status: live
- Coverage: e2e/journey

### LE-72: The primary caller surface remains eight operations (`start`, `list`, `show`, `append`, `event`, `history`, `terminate`, `invoke`). Visible `invocation-progress`, `fan-out`, and `preview-bindings` are other commands, not a ninth primary. `fan-out` and `preview-bindings` do not open the run database. `invocation-progress` opens the catalog; a query failure does not flip overlay.
- Status: live
- Coverage: e2e/journey

### LE-73: The semantic provider interface remains `describe` + `evaluate`.
- Status: live
- Coverage: e2e/journey

### LE-74: Provider correctness does not depend on retained in-memory state from earlier invocations.
- Status: live
- Coverage: e2e/journey

### 14.9 Work-Slot Delegation

### LE-75: A caller can inspect the frozen slot catalog (`work_slots`) and sparse `work_slot_bindings` from `show` / `initial_input` before work proceeds. `preview-bindings` inspects that JSON before `start` without creating a run. It reports a `dagu` PATH check (minimum 2.14.0) as ok with path and version or as a warning; well-formed bindings still exit 0. `fan-out` and `software-change run-plan-graph` execute fail-close on the same missing, unrunnable, or unsupported-version condition before any worker spawn. Isolated home is `capture_dir/dagu-home/` with locator `capture_dir/dagu-locator.json` keys `dagu_home`, `dag_name`, and `run_name` (`fanout-<capture-dir-name>` for fan-out, `plan-graph-<capture-dir-name>` for plan-graph). loop-engine and software-change packages do not contain or vendor `dagu`.
- Status: live
- Coverage: e2e/journey

### LE-76: Omitted `work_slot_bindings` and `{}` both mean no bindings; unknown slot IDs, unknown binding fields, and non-object values are rejected at `start`. `start` does not parse `fan-out` or `run-plan-graph` argv. `preview-bindings` exits nonzero on a zero-worker `fan-out` freeze.
- Status: live
- Coverage: e2e/journey

### LE-77: When a slot is bound, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body and states the bound-instruction triage order (overlay succeeded is bound CLI exit 0, not provider acceptance; captures are at the named directory; the driver triages, appends, then requests the shown event; on overrun run `show` immediately before re-invoking; on failed inspect `summary.json` and captured output before stderr).
- Status: live
- Coverage: e2e/journey

### LE-78: `invoke` is the only legal start for bound work. On accept it allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context` when the slot declared nonempty `stdin_context_kinds` (not argv, environment, or a temp file). Waiter stdin is not the worker packet.
- Status: live
- Coverage: e2e/journey

### LE-79: Hidden `wait-invocation` is parent of the bound worker, waitpids it, writes terminal `succeeded`/`failed` plus `exit_code`, then exits. After waitpid, a well-formed `capture_dir/summary.json` is copied as `inner_workers` (`command`, `args`, `exit_code` only); overlay remains the bound CLI process exit. It is not a daemon. A vanished waiter with no terminal status is overlay-`failed`.
- Status: live
- Coverage: e2e/journey

### LE-80: Hidden `stdin-exec` opens a stdin file, attaches it to child stdin, and runs `COMMAND [ARG]...` after `--` with no shell. Duty bytes stay in that file (not argv or environment). Sidecar mode writes `{"exit_code": <inner waitpid as i32>}` then exits 0; propagate mode is the inner waitpid and rejects `--sidecar-file`. Spawn failure exits nonzero without a successful sidecar. `--help` omits it. When `PI_CODING_AGENT_SESSION_DIR` is unset in the inherited environment, stdin-exec colocates Pi sessions under the worker `capture_dir/sessions` via that variable at spawn; frozen argv does not add `--session-dir`. `software-change` duplicates the same helper; plan-graph uses propagate mode only.
- Status: live
- Coverage: e2e/journey

### LE-81: Invocation records are engine-authored; `append` cannot write them. History records `invocation started {invocation_id}` and `invocation status changed {invocation_id, status}` for waiter-written `succeeded`/`failed` only.
- Status: live
- Coverage: e2e/journey

### LE-82: `work_slot_invocations.status` is the reader overlay `running` | `succeeded` | `failed` | `overrun`. Each view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers`. Overlay `overrun` is not a history action. `waiter_pid` is not in `show`. `show` does not spawn a provider and does not read capture files. While overlay is `running`, the canonical driver poll is `show` for overlay (`inner_workers` empty) plus `invocation-progress` for inner graph/traces. Graph state is Dagu helper liveness (`reaped` is helper finished, not overlay success and not inner waitpid 0). `dagu status` / `dagu history` remain the underlying surface `invocation-progress` uses, not the driver-facing path.
- Status: live
- Coverage: e2e/journey

### LE-83: A bound checked edge is refused unless overlay status is `succeeded` matching slot ID, `instruction_digest`, and the current slot-visit subject. Overlay `running`, `failed`, and `overrun` do not satisfy. Check-free edges are ungated. `evaluate` never waits.
- Status: live
- Coverage: e2e/journey

### LE-84: Overlay `overrun` is terminal for retry: a later `invoke` of the same slot is not already-running, but the driver runs `show` immediately before re-invoking. Failed and overrun records remain inspectable and never count as success. On failure the driver inspects `summary.json` and captured output before stderr. Overlay succeeded remains the bound CLI exiting 0 even when stored `inner_workers` contain a nonzero `exit_code`.
- Status: live
- Coverage: e2e/journey

### LE-85: When a slot has no binding, the driver may perform that job and no invocation record is required. When the binding set is empty, a run can still complete with the driver performing the work.
- Status: live
- Coverage: e2e/journey

### LE-86: Policy-document has no work slot for `prepare` → `ready`. Software-change, policy-document, and research share the same binding, invoke, overlay, and gate contract; each only declares its catalog.
- Status: live
- Coverage: e2e/journey

### LE-87: Slot-visit subjects are minted via set-current-subject on entry into a slot state, including `start` when the initial state is a slot. `invoke` snapshots via get-current-subject and does not mint. `instruction_digest` is SHA-256 of the stored instruction body UTF-8 bytes, lowercase hex.
- Status: live
- Coverage: e2e/journey

### LE-88: Public-boundary journeys (`scripts/software-change-journey.py`, `scripts/policy-document-journey.py`, `scripts/research-journey.py`) freeze a sparse dummy-worker binding, invoke before the bound checked event, and prove catalog snapshot, instruction redaction, unbound-invoke rejection, pre-evaluate gate, worker-packet stdin, overlay `succeeded`, unbound stored instructions, and invocation history. Software-change journeys also prove unbound shipped profiles, graph-runner and fan-out behavior with dummy inner workers, `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, `preview-bindings` warning when pi has `--no-extensions` and no `-e`, and do not call a live model. `scripts/software-change-journey.py --self-test` executes the three provider skill constructors against software-change high-rigor design-review, policy-document shipped semantic policies/target/mode, and research verify plus synthesize; it compares worker count/order and exact axis/`example_prompt`/author/model/subject metadata to each source profile, asserts required keys/data bytes/preview visibility and fail-closed invalid cases, asserts root AGENTS rules, and prints `worker-data skill/root policy assertions passed` only after all pass. The software-change source full journey binds deterministic stdin-capturing workers that emit conforming JSON or exit-0 refusal text and, through separate public CLI processes, asserts the compact one-key `artifact_root` context precedes the separator/body with no `capture_dir` or duplicate identity in that block, conforming `status` succeeded, refusal `status` failed with `exit_code` 0, summary/captures persist, overlay fails, then prints `contracted fan-out failure`.
- Status: live
- Coverage: e2e/journey

### LE-89: Bound review slots frozen to `fan-out` still require `loop-engine invoke RUN_ID SLOT_ID`. A usable review binding contains provider-constructed assigned `--worker` objects frozen at `start` after `preview-bindings` and lock-in; a review slot with an empty configured policy-axis list is not bound. Shipped profiles omit `work_slot_bindings` so slots stay driver-performed. Opt-in skill templates keep `--no-extensions` and add `-e` placeholders for cursor-provider and claude-bridge. Default implement inner argv when `--task-worker` is omitted remains `pi --print --no-skills --no-extensions` and must not pass `--no-context-files`.
- Status: live
- Coverage: e2e/journey

### LE-90: Nested fan-out workers accept only `command`, `args`, optional opaque `preamble`, and optional `output_schema` with exact required-key syntax. Bound stdin is compact absolute `artifact_root` JSON, plus `context` when the invoke packet carried matching `stdin_context_kinds` (optional preamble plus fixed separator), and does not dump `instruction_body`; the worker packet, digest, and gate matching remain unchanged. Ad hoc framing adds no run context. Contracted output records preserve process exit and capture paths, add conformance status/error, and are written to `summary.json` before facade failure. Fan-out is a local Dagu `type:graph` with concurrent worker steps that have no inter-worker depends, no `continue_on`, and no `retry_policy`; omitted `--max-active` emits no `max_active_steps` (uncapped); `--max-active N` emits `max_active_steps` N. Sidecar inner exits, mechanical join, and facade fallback if the graph stops before join remain. `software-change run-plan-graph` omitted `--max-active` remains `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks; the summarizer still runs after those tasks.
- Status: live
- Coverage: e2e/journey

### LE-91: A software-change run freezes an `operating_context` naming its operators, environment, threat boundary, accepted risks, and outside obligations. Fresh draft, review, implementation, and validation actors inspect that same context; accepted risks never waive a stated outcome or outside obligation, and excluded hostile or multi-tenant scenarios are not silently added to the trusted sole-operator boundary.
- Status: live
- Coverage: e2e/journey

### LE-92: Reviewer output is candidate evidence only. The driver owns the append-only current `finding-ledger` disposition and routing view: an advisory classification proposal is inert until accepted or edited, and only current accepted unresolved implementation findings explicitly routed to a task or review axis affect packets or later gates; raw candidate sources and driver decisions remain distinguishable.
- Status: live
- Coverage: e2e/journey

### LE-93: A bound reviewer using `full_output_schema` receives at most one correction opportunity under the identical frozen command, arguments, assignment, preamble, and model. The first and second raw stdout/stderr attempts, exact validation errors, selected attempt, and exhaustion state remain retrievable; a second invalid response fails closed without a substitute reviewer or semantic verdict.
- Status: live
- Coverage: e2e/journey

### LE-94: `implementation-ready` cannot advance from an author-declared implementation report alone. The provider-generated implementation checkpoint independently binds the report and document revisions to the current repository HEAD, index, status, tracked/non-ignored-untracked entries, and content identity, and the public checkpoint command is read-only with respect to Git.
- Status: live
- Coverage: e2e/journey

### LE-95: After implementation or final validation proof is accepted, changing repository HEAD, adding or deleting a tracked or non-ignored untracked entry, renaming an entry, changing status or type, or changing tracked bytes makes the affected checkpoint stale and refuses later checked progression until proof is regenerated. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, whether or not implementation review is configured. Validation requires the sole accepted entry for the current report revision to match the current report, document revisions, and repository state. Appending later context, overwriting history bytes, or replacing both mutable checkpoint files cannot admit bytes that transition did not accept.
- Status: live
- Coverage: e2e/journey

### LE-96: When validation exposes a stale repository checkpoint, the validation draft and review states expose a check-free `revise-implementation` recovery route. Final approval requires regenerated implementation and validation checkpoints for the same current tree plus passing review evidence; validation cannot silently replace implementation proof.
- Status: live
- Coverage: e2e/journey

### LE-97: Plan and validation review require observable user or operator outcomes, pragmatic black-box proof or a concrete impracticality reason, and semantic inspection of every new or changed Bookends citation. A final validation report maps requirements to observable proof, and passing activity or a matching requirement token alone is not treated as completion.
- Status: live
- Coverage: e2e/journey

### LE-98: `show` of current state and instructions is the observation that arms mutation. `append`, `event`, `invoke`, and `terminate` refuse without a current observation, preserve state and semantic history on refusal, and require a new observation after every state visit, including a self-loop; `list`, `history`, and `invocation-progress` do not arm mutation.
- Status: live
- Coverage: e2e/journey

### LE-99: A completed bound invocation durably identifies each enumerable assignment independently, including selected attempt or coverage gap, the digest of selected originating bytes, and their originating-attempt location. A worker-level copy is not the selected-attempt identity, and these facts remain inert until a driver acts on them.
- Status: live
- Coverage: e2e/journey

### LE-100: Ordinary `show` exposes a deterministic provider-free, fail-closed change report for assignment records and recorded plan-task results. It reports covered subject, assignment/binding, policy/configuration, output-contract, routed-input, task-definition/packet, dependency, worker-binding, and task-recorded repository-effect dimensions; unknown inputs are changed; standing records and results are visible from the durable run without provider execution or capture-file reads.
- Status: live
- Coverage: e2e/journey

### LE-101: `invoke` may select only named enumerable assignments using the existing invoke path. Empty, duplicate, unknown, or non-enumerable selections refuse before a worker starts; argv that resembles fan-out behind an executable other than the current engine remains non-enumerable; omitted selection runs the frozen binding in full; the validated selection is durable and never rewrites the frozen binding.
- Status: live
- Coverage: e2e/journey

### LE-102: `software-change run-plan-graph` may select plan-task roots plus their dependants without auto-including missing prerequisites. Invalid selections refuse before Dagu or a task starts; omitted selection remains full execution; every successful invocation still runs the summarizer and repository checkpoint against the resulting working tree, including effects left by unselected tasks.
- Status: live
- Coverage: e2e/journey

### LE-103: `unchanged-carry` on the existing append path consults the durable change report and refuses when any covered input changed. A successful carry preserves the originating author's identity and selected-output digest, records the attesting driver and carry act separately, and makes the result distinguishable from a fresh worker judgment. It attests the exact report snapshot it saw; later drift makes the contribution non-standing until another explicit act.
- Status: live
- Coverage: e2e/journey

### LE-104: `override-carry` on the existing append path is distinct from `unchanged-carry` and requires the driver to name every changed covered input. The durable run exposes the act and overridden inputs; the engine records the attestation but does not decide whether the carry was warranted.
- Status: live
- Coverage: e2e/journey

### LE-105: A software-change evidence record linked to selected originating bytes is accepted only when its invocation, assignment, selected attempt, digest, and path match the engine-selected durable record and its availability and mechanical judgment fields agree with those bytes. Fabricated identity is refused at append; missing, changed, unavailable, or disagreeing bytes are unverified and cannot satisfy a checked transition; worker invocation records alone remain inert.
- Status: live
- Coverage: e2e/journey

### LE-106: Released provider binaries retain sufficient embedded data for `data-dump`, shipped profiles, templates, and reviewer protocol, and a described/evaluated run can use that data without a checkout at runtime.
- Status: live
- Coverage: e2e/journey

## 15. Complexity Guardrails

v0.1 deliberately targets:

```text
provider semantic operations: 2
primary caller operations:    8
active states per run:        1
background services:          0
automatic workflow retries:    0
provider registry APIs:       0
compatibility APIs:           0
creation/admission checks:    0
check/validator identities:   0
engine-level policy models:    0
first-class review-result models: 0
prompt-generation subsystems:  0
review-orchestration subsystems: 0
```

A new core concept or subsystem must solve a demonstrated limitation of a reference workflow or real integration.

Early `0.x` interfaces remain evolvable rather than formally frozen.

## 16. Explicitly Deferred to Technical Design

The PRD intentionally does not decide:

- persistence technology or physical schema;
- transaction and locking implementation beyond required observable atomicity;
- internal versions, sequences, or concurrency tokens;
- identifier and timestamp representation;
- provider subprocess versus another stateless integration mechanism;
- provider request/response framing and serialization;
- timeout or cancellation mechanics;
- provider-association representation;
- exact CLI grammar, JSON field names, or exit codes;
- SDK architecture;
- internal language/module/crate boundaries;
- logging, tracing, and diagnostics implementation;
- indexing, pagination, backup, or packaging mechanics.

The technical design should select the simplest implementation that satisfies the product semantics above.

## 17. Deferred Product Capabilities

The design should leave room for, but v0.1 should not implement without demonstrated need:

```text
cloud API/service
executor assignments and active attempts
user questions and approvals
live steering interruption
claims and leases
parallel work
child workflows
first-class artifact revisions and dependencies
first-class decisions/directives
workflow composition
provider distribution
provider pinning
workflow migration
remote persistence
richer audit facilities
generalized context-revision concurrency
```

These should extend or layer around the durable workflow kernel rather than move workflow authority into an agent harness.

## 18. Product Test

When considering additional complexity:

> **Is this required to durably coordinate externally performed work, or are we beginning to rebuild a general workflow platform?**

Loop Engine remains the primitive coordination kernel until concrete workflows prove that it needs to become more.
