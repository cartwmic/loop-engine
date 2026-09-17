# Loop Engine v2 — Product Requirements Document

**Status:** Living
**Target:** Current 0.x product; dated v0.1 scope is retained as history
**Compatibility:** Clean-slate successor; no v1 compatibility requirement
**Amended:** 2026-09-11

> **Recovery amendment draft (plan r5): owner acceptance and commit pending.** The recovery wording integrated below and the explicitly marked requirement amendments are proposals against the implemented contract, not a claim of accepted policy or completed audits. Existing titles and amendment history are retained; LE-51 remains tombstoned. Historical runs retain their original obligations. The owner must accept the exact diff; the driver completes document audits before final proof.

> **Backlog amendment scope:** The historical-scope labels and changes to execution description, context-only evaluation, LE-100/102/110/116/119/125, replacements LE-128/129/130/137/138 and additions LE-131–136 are one scoped amendment. They do not accept unrelated recovery-r5 proposals or reinterpret historical runs. LE-135 and LE-136 backfill intended behavior already present in the current product.

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

### 3.2 Historical v0.1 non-goals

This original scope is retained as history. Later accepted requirements govern capabilities added since v0.1. Marked pending amendments remain pending.

The original v0.1 scope excluded:

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

A **checked** transition normally requires provider `allow` before it may commit. Explicit owner-attested override is a distinct exceptional transition outcome, never a provider allow.

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
future binding amended
explicitly overridden transition
```

Cancellation request, controller-attempt and cleanup details are retained in engine-owned ownership receipts and exposed by invocation inspection. They are not separate semantic history actions; verified cancellation acknowledgment records the terminal invocation-status change.

Invocation terminal status is engine-written `succeeded` or `failed`. The waiter records ordinary completion; admitted cancellation requires the controller's verified cleanup acknowledgment before terminal failure. Overlay `overrun` is a reader projection on `show` (`work_slot_invocations.status`) and is not a history action. `invoke` rejections (unknown slot, unbound slot, already-running) and waiter/worker spawn failures remain operation results, not semantic history.

A transition history entry exposes the relevant:

```text
event
source state
target state
checked/check-free
outcome: committed | denied | overridden
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
6. **A check-free transition needs no provider evaluation.** Observation, active lifecycle and the live-owned-work barrier still apply; no route authorizes overlapping writers.
7. **Rejected or errored event requests leave current state unchanged.**
8. **A committed transition and its required history entry are atomic.**
9. **A durable checked-transition denial and its feedback are recorded atomically.**
10. **Current state is authoritative; history replay is not required.**
11. **Primary workflow work remains external to Loop Engine.**
12. **Actor type and harness do not change workflow authority.**
13. **v0.1 assumes one logical mutating actor per run.** Accidental concurrent processes must not corrupt or commit conflicting workflow state.

## 6. User-Facing Operations

The proposed recovery surface contains ten primary operations. Exact CLI spelling remains evolvable during v0.x.

| Operation | Purpose |
|---|---|
| `start` | Create a run from a provider and initial input |
| `list` | Discover runs |
| `show` | Obtain the current working context |
| `append` | Add a context record |
| `event` | Request workflow progress |
| `history` | Inspect semantic run history |
| `terminate` | Explicitly close an active run |
| `invoke` | Preview or start a bound work-slot worker |
| `amend-binding` | Owner-attested future execution correction |
| `cancel-invocation` | Stop and verify cleanup of current invocation-owned work |

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

When object initial input contains `work_slot_bindings`, the engine freezes the initial map from slot ID to `{command, args, context_filter?}`. An optional filter is `{command,args}`. Ordered owner-attested amendments derive effective future bindings without rewriting this initial map. Omitted key and `{}` both mean no bindings. Start rejects an unknown slot ID (not in the provider `describe` catalog snapshot for this workflow), unknown fields on a binding object, and non-object values. Start does not parse `fan-out` or `run-plan-graph` argv. If the initial state is a work slot, the engine mints a **slot-visit** subject for that visit via set-current-subject.

### 6.2 Show

`show` is the primary resumption and actor interface.

Full inspection (`show --view full`) exposes the information below. Ordinary `show` provides the focused current-action view and references to omitted material; status-only observation does not arm mutation, as specified in LE-20.

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
complete evaluation_history with original identities and ordering
work_slots (catalog snapshot: id, state, event, optional stdin_context_kinds; no instruction body)
work_slot_invocations (invocation_id, slot_id, binding snapshot, optional assignment_selection, instruction_digest, subject, overlay status, overlay_meaning, elapsed_ms, remaining_allowed_ms, capture_dir, inner_workers, started_at, allowed_time_ms, optional exit_code, optional completed_at)
```

`work_slot_invocations.status` is the reader overlay result `running` | `succeeded` | `failed` | `overrun`, not a raw waiter-written row when overlay applies. `waiter_pid` is internal and is not in `show`. Each invocation view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers` (`command`, `args`, `exit_code` in argv or task order after the bound CLI finishes; empty while overlay is `running` or when no summary was copied). Completed invocations additionally project a durable provider-free change report: subject revision, assignment and binding, run identity, declared output contract, and routed inputs for assignments; and task definition/packet, dependencies, routed inputs, worker binding, and the task-recorded repository effect for plan-task results. The run-level `change_report` exposes `assignments` and recorded `plan_task_results`. The former `change_report.judgments` show field is tombstoned: `assignments` contains the same generic assignment records under the renamed public key; provider reviewer judgments remain provider content, not this schema. Unknown report inputs are changed. `show` remains provider-free and reads engine-owned ownership/cancellation metadata under `capture_dir`; semantic interpretation of worker output remains the driver's duty. Overlay meaning: succeeded means the bound CLI exited 0, not that the provider accepted the work; failed means the bound CLI exited nonzero or the waiter vanished; running means the waiter is alive and allowed time has not elapsed; overrun means allowed time elapsed while the waiter is alive; wait or cancel owned work and verify cleanup, then observe before retry. Missing waiter liveness is not cleanup proof. When the current state is a bound slot, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body. Bound-instruction triage order: overlay succeeded means the bound CLI exited 0, not that the provider accepted the work; captures are at the named capture directory on the invocation view and invoke result; the driver triages worker output, appends provider-shaped records, then requests the shown event; on overrun wait or cancel owned work and verify cleanup, then observe before retry; on failed inspect `capture_dir/summary.json` and captured stdout before stderr. Do not redact to only the invoke CLI. Unbound current states keep the stored instruction body.

The `latest_evaluations` projection is the **chronologically latest durable evaluation** for each exact checked transition. An `allow` supersedes any earlier `deny`, and a later `deny` likewise supersedes an earlier `allow`. When the latest result is `deny`, its actionable feedback is exposed.

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

> **Non-normative stable-reference note:** New append records use concise context-record or invocation/assignment references. Core resolves same-run opaque invocation identities and records rich output metadata once; the software-change provider resolves its own subject, evidence, finding, task, policy, and checkpoint identities. Review reuse uses one explicit `evidence-applicability` declaration with a current target, attesting driver, and short reason. Completed historical runs may still expose the former verbose records, but those records are not new ordinary append forms.

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
3. if the edge is a **bound** work slot (present in frozen `work_slot_bindings`), requires an overlay-`succeeded` invocation matching slot ID, `instruction_digest` (SHA-256 of the stored instruction body UTF-8 bytes, lowercase hex), and the current **slot-visit** subject from get-current-subject; overlay `running`, `failed`, or `overrun` do not allow the normal edge; this gate runs before provider `evaluate` and never waits. All departures first require quiescent owned work; check-free edges omit provider and bound-success checks. An explicit override may skip those checks only after quiescence;
4. for a check-free transition, atomically commits the target state and one aggregate history entry without invoking the provider;
5. for a checked transition, constructs the evaluation request and asks the provider to evaluate that exact transition;
6. after the provider returns `allow` or `deny`, verifies that the run is still active and remains in the source state against which evaluation began;
7. if that state/lifecycle check fails, treats the evaluation as stale, returns an error, and records no semantic history or evaluation lineage from the stale result;
8. on a non-stale `allow`, atomically commits the target state and one aggregate history entry, and mints a new slot-visit subject via set-current-subject when the target is a work slot (replace);
9. on a non-stale `deny`, preserves state and atomically records one aggregate denied-transition history entry containing the feedback;
10. on `unsupported` or operational failure, preserves state and returns an error without adding semantic run history.

Context-only appends do not invalidate an in-flight evaluation. Its original request remains unchanged; later evaluations receive the appended context. State-visit and lifecycle changes retain their staleness checks. Drivers serialize workflow mutations; the explicit context-append interleaving is covered by LE-128.

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

`terminate` requires a current `show` observation and quiescent invocation-owned work, then closes an active run without provider evaluation and records one termination history entry. Live work, including overrun or pending cleanup, rejects termination without changing lifecycle or semantic history; wait or cancel and verify cleanup before terminating. An unobserved termination is also rejected.

Final and terminated runs cannot be terminated again.

A terminal `terminate` request is rejected and creates no semantic history.

A terminated run cannot reopen in v0.1.

### 6.8 Invoke

Invocation requires current observation and an effective bound slot. It freezes the admitted command, controls, selected inputs and current work identity, and retains execution and capture identities for later inspection. Preview performs preparation without admitting work. Invalid selected work must be refused before it can displace reusable results.

Live owned work and pending cleanup block retry and departure, including after an allowance expires or a waiter disappears. Recorded process numbers alone cannot establish ownership of a later process. Cancellation acts only on established ownership, preserves output, prevents later task admission and reports verified cleanup or an explicit incomplete result.

A terminal process outcome records the actual bound command's exit. Output conformance and semantic acceptance remain separately visible. Helper placement and parentage may change while preserving these outcomes. LE-129 defines the execution obligation; the CLI specification describes the helper arrangement.

Full plan execution and selected-task execution retain successful applicable prerequisites. A selected root includes its dependants. A later real replacement may invalidate earlier success. Missing evidence remains distinct from an optional effect omitted by both otherwise present records. The driver owns semantic applicability and the existing checkout.

The [CLI specification](agent-usage.md#work-slot-delegation) owns argument grammar, packet fields, capture layout, helper modes and session placement. These details remain documented and tested. This section imposes no additional helper topology.

### Other command: invocation progress

Progress inspection reports available execution status and trace locations without modifying workflow state. Helper completion, worker exit, conformance and semantic judgment remain distinct. Missing or conflicting information stays explicit. Deterministic monitoring supplies passive completion/attention notifications under LE-120.

### Non-run-state execution: fan-out and plan graphs

Fan-out executes caller-declared worker commands with retained inputs, outputs and mechanical conformance results. It does not create a workflow run or advance a gate. The engine owns scheduling, capture, selected execution and bounded conformance correction. Providers own assignment framing; external reviewers own judgment.

An optional execution barrier can order two worker groups. A conforming semantic failure does not prevent a later independent review group from running. A required execution or conformance failure remains visible and cannot establish successful completion. Selection preserves original assignment identities.

Plan-graph execution uses the driver's existing repository, preserves dependencies and standing prerequisites, and writes a current report/checkpoint after successful selected work. The existing no-task repair path remains limited to an accepted implementation finding with no honest frozen task owner. Neither path manages worktrees or chooses semantic recovery routes.

Dagu remains an operator-provided subprocess dependency. Its Go API is not embedded, and release packages do not ship Dagu. Supported command forms, dependency checks and capture layouts are maintained in the [CLI specification](agent-usage.md#non-run-state-command-fan-out).

### Non-run-state setup inspection

Binding preview exposes the effective commands, declared worker contracts and dependency diagnostics before start. It creates no run. Malformed inputs and empty worker definitions fail closed. Warnings remain distinct from execution readiness and semantic approval.

The software-change setup utility produces a per-run profile from explicit rigor and roster inputs under LE-132. The owner confirms its effective policy and commands before start. Frozen runs retain their original policy; changing source defaults does not change an existing run.

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

Providers cannot route to another state through `evaluate`. An `allow` may include an optional opaque `context_append` effect (`kind` and `data`), which the engine persists atomically with the exact checked transition; this does not grant provider routing authority or engine-level truth semantics to the context.

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

Context-only appends preserve the in-flight evaluation snapshot and do not invalidate its allow or deny. Later evaluations receive the appended records in order, as required by LE-128.

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
  ├─ implementation-ready [checked] → implementation-review
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

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

Shipped software-change profiles omit `work_slot_bindings` (or `{}`), so draft slots stay driver-performed by convention. Review slots are bindable. Bound workers are opt-in skill templates: keep `--no-skills --no-extensions`, add an existing `-e` extension path for each extension required by the selected model provider, name `--model MODEL`, and fill those values in the per-run profile JSON. An omitted extension path contributes no `-e` pair. A usable review binding is caller-supplied `--worker` objects frozen at `start` after `preview-bindings` and lock-in. Documented review `pi` worker examples include `--print --no-skills --no-extensions --tools read,grep,find,ls`; optional `-e` paths are validated when supplied, and workers must not pass `--no-context-files`. `preview-bindings` warns when a pi worker has `--no-extensions` and no `-e`; missing `--no-extensions` is not a required warning. Policy-document and research shipped profiles stay unbound.

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
        on overlay overrun, wait or cancel owned work and verify cleanup;
          then run show before retry or departure
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

Ordinary `show` provides current action instructions; separate status-only and full inspection views remain available. Status-only observation does not arm mutation. Action and full instruction reads arm the current visit. Optional provider-authored action guidance remains opaque to core; absent legacy guidance means unknown normalized obligations, not zero obligations, and preserves the existing bound-invocation or external-work path and access to frozen policy. Full inspection retains complete configuration, context and invocation/change-report evidence with original identities and ordering. The focused action view exposes current obligations, active work, source-located blockers with explicit freshness or uncertainty, and references for omitted material; unrelated completed invocations and historical context do not enlarge it.

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

Full inspection also exposes every durably recorded checked allow/deny evaluation in original semantic-sequence order, preserving its identity, exact transition, feedback, sequence and occurred_at. This complete evaluation history is available in full inspection itself, alongside the unchanged latest-per-transition projection. Overrides remain separate history, not synthetic provider evaluations; unrecorded operational failures are not fabricated as evaluations.

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
- Status: tombstone

Superseded by LE-128. The original title is retained for continuity and historical interpretation.

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
- Status: tombstone

Superseded by LE-137. The original title is retained for continuity and historical interpretation.

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
- Status: tombstone

> **Proposed retirement, recovery r5:** LE-119 replaces the eight-operation count with two explicit execution-control operations. The original title remains historical.

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
- Status: tombstone

Superseded by LE-129. The original title is retained for continuity and historical interpretation.

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
- Status: tombstone

> **Proposed retirement, recovery r5:** elapsed allowance cannot authorize overlapping live work. LE-110 replaces retry permission; history and capture inspection remain required.

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
- Status: tombstone

Superseded by LE-139. The original title is retained for continuity and historical interpretation.

### LE-89: Bound review slots frozen to `fan-out` still require `loop-engine invoke RUN_ID SLOT_ID`. A usable review binding contains provider-constructed assigned `--worker` objects frozen at `start` after `preview-bindings` and lock-in; a review slot with an empty configured policy-axis list is not bound. Shipped profiles omit `work_slot_bindings` so slots stay driver-performed. Opt-in skill templates keep `--no-extensions` and add `-e` placeholders for cursor-provider and claude-bridge. Default implement inner argv when `--task-worker` is omitted remains `pi --print --no-skills --no-extensions` and must not pass `--no-context-files`.
- Status: tombstone

Superseded by LE-140. The original title is retained for continuity and historical interpretation.

### LE-90: Nested fan-out workers accept only `command`, `args`, optional opaque `preamble`, and optional `output_schema` with exact required-key syntax. Bound stdin is compact absolute `artifact_root` JSON, plus `context` when the invoke packet carried matching `stdin_context_kinds` (optional preamble plus fixed separator), and does not dump `instruction_body`; the worker packet, digest, and gate matching remain unchanged. Ad hoc framing adds no run context. Contracted output records preserve process exit and capture paths, add conformance status/error, and are written to `summary.json` before facade failure. Fan-out is a local Dagu `type:graph` with concurrent worker steps that have no inter-worker depends, no `continue_on`, and no `retry_policy`; omitted `--max-active` emits no `max_active_steps` (uncapped); `--max-active N` emits `max_active_steps` N. Sidecar inner exits, mechanical join, and facade fallback if the graph stops before join remain. `software-change run-plan-graph` omitted `--max-active` remains `max_active_steps` 4 ordinary plan tasks; `--max-active N` is at most N ordinary plan tasks; the summarizer still runs after those tasks.
- Status: tombstone

Superseded by LE-138. The original title is retained for continuity and historical interpretation.

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

`show` exposes a deterministic provider-free, fail-closed change report for assignment records and recorded plan-task results. Full inspection reports covered subject, assignment/binding, policy/configuration, output-contract, routed-input, task-definition/packet, dependency, worker-binding, and task-recorded repository-effect dimensions; unknown inputs are changed; standing records and results are visible from the durable run without provider execution or capture-file reads. Ordinary action inspection exposes selected relevant change-report facts and references to full inspection rather than unrelated historical reports.

For two otherwise present task records, an optional repository effect omitted by both is a known equal absence. A missing required record or dimension remains unknown and changed. This distinction cannot revive a result replaced by a later real failed execution.

### LE-101: `invoke` may select only named enumerable assignments using the existing invoke path. Empty, duplicate, unknown, or non-enumerable selections refuse before a worker starts; argv that resembles fan-out behind an executable other than the current engine remains non-enumerable; omitted selection runs the frozen binding in full; the validated selection is durable and never rewrites the frozen binding.
- Status: live
- Coverage: e2e/journey

Here, selection means engine-owned `--assignment`/`--assignments` selection. A bound invocation may additionally accept one distinct optional opaque JSON value through `invoke --input`; core validates only JSON framing, rejects `--input` together with assignment selection, durably stores and transports the exact value, preserves the frozen command and args, and does not interpret provider semantics. Omitted input remains full execution.

### LE-102: `software-change run-plan-graph` may select plan-task roots plus their dependants without auto-including missing prerequisites. Invalid selections refuse before Dagu or a task starts; omitted selection remains full execution; every successful invocation still runs the summarizer and repository checkpoint against the resulting working tree, including effects left by unselected tasks.
- Status: live
- Coverage: e2e/journey

Direct `--task`/`--tasks` selection and bound software-change implement invocations may use their respective selection forms; the bound path interprets `invocation_input` exactly as `{plan_revision,task_roots}`. It rejects malformed, empty, duplicate, unknown, stale-plan, missing-prerequisite, or ambiguous selection before Dagu, any plan-task worker, the summarizer, or repository checkpoint starts. A valid selected invocation requires each prerequisite to be selected or already standing for the same plan revision under the existing durable projection; roots include transitive dependants. Omitted direct selection and omitted input remain full execution.

An invalid retry refused before any task starts must preserve previously standing prerequisite results. Successful selected recovery reuses only currently applicable prerequisites and retains the required summarizer/report/checkpoint outcome.

### LE-103: `unchanged-carry` on the existing append path consults the durable change report and refuses when any covered input changed. A successful carry preserves the originating author's identity and selected-output digest, records the attesting driver and carry act separately, and makes the result distinguishable from a fresh worker judgment. It attests the exact report snapshot it saw; later drift makes the contribution non-standing until another explicit act.
- Status: tombstone

> **Non-normative historical note:** Completed runs may expose `unchanged-carry` records from the former contract through immutable inspection. New appends use `evidence-applicability`; no migration or compatibility path is provided.

### LE-104: `override-carry` on the existing append path is distinct from `unchanged-carry` and requires the driver to name every changed covered input. The durable run exposes the act and overridden inputs; the engine records the attestation but does not decide whether the carry was warranted.
- Status: tombstone

> **Non-normative historical note:** Completed runs may expose `override-carry` records from the former contract through immutable inspection. New appends use `evidence-applicability`; no migration or compatibility path is provided.

### LE-105: A software-change evidence record linked to selected originating bytes is accepted only when its invocation, assignment, selected attempt, digest, and path match the engine-selected durable record and its availability and mechanical judgment fields agree with those bytes. Fabricated identity is refused at append; missing, changed, unavailable, or disagreeing bytes are unverified and cannot satisfy a checked transition; worker invocation records alone remain inert.
- Status: live
- Coverage: e2e/journey

**Stable-reference delivery note:** The caller now supplies only a same-run invocation/assignment origin reference. Core resolves the selected attempt, digest, path, capture, command, and binding from durable engine state; the provider performs the existing byte and judgment-field checks. This delivery does not add driver-authored provenance duplication under LE-107.

### LE-106: Released provider binaries retain sufficient embedded data for `data-dump`, shipped profiles, templates, and reviewer protocol, and a described/evaluated run can use that data without a checkout at runtime.
- Status: live
- Coverage: e2e/journey

### LE-107: Minimum provenance is a paramount product constraint.
- Status: live
- Coverage: e2e/journey

> **Amendment history — this proposal was accepted through owner-delegated artifact review on 2026-08-26; its authority is established only by a separate commit.** For Loop Engine and the software-change provider, assume a trusted sole owner and well-intentioned, instruction-following drivers. Provenance serves only fresh resume, focused invalidation, honest bypass, debugging, and visible history. Capture each fact once under a stable engine-owned identity and reference that origin; keep ordinary driver-authored metadata small. Trust explicit driver declarations of materiality and evidence applicability except for cheap mechanical identity mismatches. Prefer the narrowest honest correction, preserve valid work, and do not replay still-valid work merely for ceremony. Actual failures, findings, overrides, evidence, checkpoints, and terminal history remain visible. Every proposed provenance field, identity dimension, gate, state, or replay rule must identify the observed ordinary-use failure it prevents and explain why existing durable state, history, capture, or driver judgment is insufficient. This preserves LE-18 and the engine/caller/provider ownership boundary: core assigns context no truth or provenance, callers perform work externally, and providers evaluate the exact selected transition without routing or judging semantic truth. It adds no engine truth model, gate, state, replay system, or provenance framework.

> **Proposed LE-107 amendment, recovery r5 — owner acceptance pending:** Simple-first, YAGNI and KISS govern Loop Engine as a whole and every provider it generates, ships or uses. Start with the simplest adequate solution. Added complexity requires a documented meaningful current requirement or observed ordinary-use failure and a brief explanation, in the ordinary design, of why an adequate simpler approach is insufficient. Gold-plating, productionization for its own sake, speculative defense-in-depth and adversarial guards without that basis are not wanted. Current implementation, architecture, communication protocols, schemas, dependencies and internal mechanisms have no presumption of preservation. A simpler replacement or removal is in scope when it meaningfully reduces complexity, scope, footguns or ordinary-use failures; change for novelty and compatibility scaffolding solely to preserve incidental design are not justified. Keep driver-authored provenance small and capture mechanical facts once, while retaining honest failures, findings, exceptions and rich engine history. Trust explicit materiality/applicability declarations except cheap identity mismatches. Review judges the ordinary design and delivered outcomes; no separate justification framework, gate or metadata inventory is required. Policy-document and research gain common simplicity/execution guidance, not software-change-specific finding or criterion features.

### LE-108: A bound software-change implementation can capture one focused no-task repair without replaying valid plan work.
- Status: live
- Coverage: e2e/journey

> **Amendment history — accepted for Package 5 through owner-delegated intent, design, and plan review on 2026-08-28.** The observed failure is an accepted current implementation finding for which no frozen plan task honestly owns the correction: before this amendment the driver had to revise an otherwise-correct plan, replay unrelated work, make an uncaptured off-engine correction, or strand the run. On the existing bound `implement` slot, opaque invocation input may therefore be exactly `{"repair_finding_ids":[...]}`, disjoint from omitted full execution and exact `{"plan_revision":"...","task_roots":[...]}` selection. The provider accepts only unique named findings from current engine-forwarded ledger snapshots that are accepted, unresolved, implementation-owned, current for the subject and verified implementation checkpoint, and carry empty `task_ids`; malformed, empty, unknown, stale, wrong-owner/status/disposition, or task-routed requests refuse before Dagu resolution, proof deletion, worker launch, or repository mutation. A valid request uses the unchanged frozen worker and checkout for exactly one captured `ad-hoc-repair` assignment, with the selected finding objects and provider-derived pre-repair proof identity; it runs no plan task or summarizer and does not alter `plan-task-results.json`. The worker must write a schema-valid implementation report linked to the frozen plan whose report revision is unused by both the immediately preceding proof and every accepted implementation-proof-history entry. Only then does the provider create a new implementation checkpoint. The ordinary invocation and summary retain the exact input, frozen binding, selected output, routed findings, and provider-derived pre/post report and repository-state identities; process success is not semantic acceptance, and existing independent implementation review, validation, and terminal gates still apply. Task-owned defects use LE-102 selection, materially wrong decomposition revises the plan, and no direct repair flag, core finding semantics, automatic task-fit judgment, rollback, carry redesign, binding correction, generalized replay, criterion redesign, or progress/status redesign is added.

### LE-109: A completed software-change `show` can be piped to `software-change review-candidates` for a deterministic, provider-owned view of bound review assignments. The view exposes only normalized selected-output judgment fields with a stable invocation/assignment origin, or a mechanical `malformed`, `unavailable`, `missing-selection`, or `exhausted` diagnostic. It does not retry, deduplicate across durable invocations, mutate captures or run state, append evidence, or satisfy a gate; after inspecting and triaging it, the driver must explicitly accept, edit, or reject the candidate and use the ordinary review-evidence and finding-ledger append path before requesting the checked event.
- Status: live
- Coverage: e2e/journey

### 14.10 Proposed recovery amendments (owner acceptance pending)

The following exact qualifications amend existing live records without renaming their titles: LE-6 and LE-83 describe **normal** checked progression; LE-113 adds the explicit exceptional mode. LE-5 and LE-54 remain provider-free but subject to LE-110's live-work barrier. LE-22 visibility gains the delivered recipient selection in LE-114, not revival of LE-51. LE-46's failed evidence blocks when not discharged by LE-115; a discharged fail is not a pass. LE-75–LE-78 and LE-101 retain initial settings but expose/snapshot effective future settings under LE-111. LE-79 and LE-81 describe ordinary waiter completion; cancellation finalization belongs to LE-112 and missing waiter liveness is not cleanup proof. LE-82 retains its overlay vocabulary, never retry authority. Its no-capture-read clause is qualified: provider-free `show` reads engine-owned ownership/cancellation metadata under `capture_dir`; worker-output interpretation remains the driver's duty. LE-88–LE-90 retain transport and distinct axis obligations while LE-118 changes software-change's default constructor allocation. LE-92 uses exact-source dispositions, not text-set equality. LE-95–LE-97 require normal accepted implementation-proof history and LE-116's final index; override does not fabricate them. LE-98 observation also arms amend-binding and cancellation admission; an outstanding cancellation can be resumed. LE-109 expands grouped selected output into inert per-axis and criterion/goal candidates. Older amendment scope exclusions describe those earlier changes, not a prohibition of this explicit amendment.

### LE-110: Live invocation-owned work blocks retry and state departure until completion or verified cleanup.
- Status: live
- Coverage: e2e/journey

Overrun is elapsed allowance, not retry permission. The barrier also blocks termination so an inactive run cannot strand its owned work. Wait or cancel, verify cleanup, then observe again. Inspect failed captures before retry; process success is not provider acceptance. New software-change implement graphs expose check-free revise-plan, revise-design and revise-intent without a report for rejected work. The driver selects the owner; no repository rollback or automatic invalidation occurs. Old stored graphs gain no edges.

For new runs, an unrelated holder of an old process or group number neither blocks progression nor receives cancellation signals. Genuine surviving owned work retains the barrier. Missing ownership evidence does not authorize signaling; historical runs are not migrated.

### LE-111: Future execution corrections preserve frozen policy and past effective attempts.
- Status: live
- Coverage: e2e/journey

An observed current visit permits owner-attested amend-binding for one catalog slot using state_visit, owner, reason and a closed replacement binding. It changes future command/args/context_filter only, not initial input, topology, schemas, semantic policy or past attempts; an already-started attempt retains its commission. Show exposes original and effective bindings; invoke preview resolves the same preparation without primary launch. Ordinary max_active, force_fresh, timeout and applicable task/review selection require no amendment. Unsupported controls refuse. Fresh execution creates new captures without implicit standing reuse; selected tasks cannot claim unselected prerequisites ran fresh, and force-fresh review cannot use carried rows. This does not erase an arbitrary worker CLI's private sessions.

### LE-112: Cancellation stops and verifies cleanup of only the current invocation-owned local process tree.
- Status: live
- Coverage: e2e/journey

Cancel-invocation validates run, current visit and recorded ownership, not arbitrary caller PIDs. Ownership is published behind a worker-start barrier; a durable stop marker under shared admission locking prevents later task/summarizer launch. The controller directly requests Dagu/direct-worker shutdown, escalates and verifies process disappearance/reaping before acknowledging terminal invocation failure. The run neither advances nor terminates and available captures survive. Each control acquisition/resumption has one monotonic ten-second deadline, at most three seconds graceful shutdown; phases do not reset it. Controller interruption remains incomplete: operator delay is unbounded, already-running work may persist, and later admission remains blocked. Retry resumes the same request with a new recorded ten-second attempt, never relabeling earlier interruption/timeout as success. Unverified cleanup keeps the barrier. Waiter loss does not defeat recorded ownership. Wrong-run/nonrunning targets without outstanding cancellation refuse without mutation; historical missing ownership is unsupported, not cancellable by assumption.

### LE-113: Owner-attested exceptional progression remains permanently distinct from normal completion.
- Status: live
- Coverage: e2e/journey

Event --override names the current state_visit, owner, reason and one available edge after observation and quiescence. Stale/malformed/unavailable requests refuse without override history. Durable outcome overridden retains attestation, exact edge, known skipped bound checks and provider evaluation as not performed (not applicable for check-free). It does not invent unseen checks, artifacts, reviewer passes or Bookends GREEN, or rewrite denials and failures. Later edges require their own proof or separate exception. Show/list/history expose permanent has_overrides/count; final lifecycle stays final with completion_mode completed-with-overrides rather than completed.

### LE-114: Applicable durable software-change steering reaches only its named later commissions.
- Status: live
- Coverage: e2e/journey

A shared provider selector supports unbound commission inspection, opaque bound context filtering and exact plan-task packets. User-steering names all, slots, or current-plan tasks; explicit whole-record supersession and append order replace inferred merging. Unknown recipients/references refuse, stale task revisions are visible and not applied, and task instructions do not spread to dependants or summarizer. A launched attempt retains its original selection. Execution owner/argv updates name existing proof obligations and a reason without revising unchanged outcomes, decomposition or proof. Unbound incorporation records are testimony, not proof of obedience; public proof must show delivered steering changing work.

### LE-115: Reasoned exact-source finding dispositions discharge failures without rewriting judgments or weakening author floors.
- Status: live
- Coverage: e2e/journey

Software-change counts distinct current independent non-retired judgments: a pass or a fail explicitly rejected/resolved by its exact evidence identity. Discharged failures are satisfied-by-disposition, not passes. Undispositioned fails and accepted-unresolved findings block with source/author/remedy diagnostics; a revision bump does not resolve the latter. Historical resolved sources/routing remain inspectable without false applicability. Reasoned retired-author disposition requires recorded gate roster change showing departure and replacement coverage; retired authors do not count. Source/capture mismatches remain mechanical denials; semantic disposition belongs to the driver.

### LE-116: Final software-change validation indexes complete independent current-criterion and whole-intent evidence at one checkpoint.
- Status: live
- Coverage: e2e/journey

Supporting software-change contracts use current frozen AC-N identities and independent criterion_policy, separate from review-axis author counts. The accepted plan names runnable proof_commands and owners; command captures retain actual argv/cwd, exit, elapsed time, output and repository identity. Failed, missing or incomplete proof cannot pass. The fixed report indexes command evidence, exactly one selected verdict set per criterion and a separate goal judgment; omissions, duplicates, unknown/stale/self-authored/unsupported evidence refuse. Prechosen unused record IDs are only names: checkpoint the index before genuine append --record-id judgments, with no placeholders or reservations. Validation-ready may leave verdicts pending for live review; approval or reviewless draft-to-end requires completeness. Ordinary review uses the retained command collection; challenge consumes the completed criterion/goal collection without recommissioning it. In high ordinary validation, individual-axis workers return axes only, while the aggregate authors alone produce their criterion/goal rows and all-axis judgments together. Pending verdict IDs contain no evidence. Approval requires the actual complete current collection. Minimal/standard retain their ordinary combined-output pattern. After repair name affected criteria and supply fresh verdicts; explicitly carry unaffected original evidence to the current report/checkpoint with driver/reason and visible original author/result. Material repair requires fresh goal judgment; only explained report-index-only correction may carry it. Unresolved criterion failures block under exact-source dispositions. Normal validation retains accepted implementation-proof-history for the same tree. Explicit driver-added `validation-command` records may strengthen the effective command collection using new distinct IDs; they cannot replace frozen required IDs, waive proof or create acceptance criteria. `proof_updates` remains execution correction for existing IDs, not an addition path. The complete index includes declared supplemental commands and their real selected execution evidence; missing, failed, stale or incomplete evidence is not passing proof. Validation preparation is inert: it may reuse applicable retained execution to prepare command-evidence candidates, an index draft and independent criterion/goal commissions, but does not execute proof, append evidence, checkpoint, issue judgments or progress the run. Repository report receipts and native provider/checkpoint identities remain distinct and are checked under their existing contracts. The driver inspects and finalizes/checkpoints the index before commissioning the required independent judgments.

### LE-117: Focused workers and bounded isolated proof jobs preserve complete final proof without duplicated suite ownership.
- Status: live
- Coverage: e2e/journey

Workers run assigned focused checks; one designated proof owner runs the complete serialized final stable-tree matrix and repeats only invalidated checks. Reviewers consume retained results rather than rerunning suites. The public journey uses one explicit jobs budget (default two, serial one) only for independent isolated work, preserves dependent run ordering and full inventory, prebuilds binaries, uses private targets for actual compiling jobs, propagates failure and verifies descendant cleanup. Repeated comparable measurements must show targeted lower wall time, with clippy/source journeys and available launch/retry/token/cost observations reported separately, never an invented baseline or universal speedup. Hosted exact-commit proof and later real dogfood remain pending until observed.

### LE-118: Shipped software-change review construction defaults to one commission per used author per gate with distinct axis verdicts.
- Status: tombstone

Superseded by LE-130. The original title is retained for continuity and historical interpretation.

### LE-119: The public recovery surface adds amend-binding and cancel-invocation without adding provider semantic operations.
- Status: live
- Coverage: e2e/journey

The ten primary operations are start, list, show, append, event, history, terminate, invoke, amend-binding and cancel-invocation. Invocation-progress, fan-out and preview-bindings remain other commands with their existing catalog boundaries. Describe/evaluate remain the provider semantic interface; provider utilities perform no semantic judgment. New supporting software-change profiles declare semantic contract version 3 and the rigor/criterion policies in LE-130. Exact profile-version strings and encodings are maintained with the shipped data. The new provider explicitly refuses older semantic contracts; retain fixed old providers for old execution. Provider-free historical reads preserve original obligations and evidence meaning, with absent ownership/control metadata treated as absent capability. No active-run migration, compatibility scaffold or bootstrap rewrite is required.

### LE-120: A provider-agnostic Loop Engine monitoring command exposes ongoing run and external-work status and completion/attention notifications without model polling or workflow authority.
- Status: live
- Coverage: e2e/journey

Status distinguishes workflow, helper execution, worker outcome, output conformance and semantic judgment or its absence; missing, stale and conflicting evidence stays explicit. The same generic public interface supports different providers, run invocations, bound/unbound graphs and fan-out, and validation commands. A waiting caller can receive machine-consumable completion/attention notifications. Observer restart preserves execution/evidence and observation never approves, advances, retries or cancels work.

### LE-121: Drivers can execute and capture serial proof matrices or one command with preserved failures, verified abort cleanup and explicit valid-prefix resume.
- Status: live
- Coverage: e2e/journey

The executor streams output and retains actual execution evidence, stops later admissions on failure, and never relabels earlier attempts. Resume refuses stale execution identity, missing evidence and unresolved cleanup. Execution success is not semantic approval; existing test commands remain the proof implementations.

### LE-122: Optional iterative semantic status summaries remain evidence-grounded, cost-bounded and separate from deterministic monitoring and workflow authority.
- Status: live
- Coverage: e2e/journey

An operator-configured external completion command receives selected evidence/source locators and a fallible previous summary, explains changed evidence, explicitly corrects errors, retains prior outputs and discloses uncertainty and available usage/cost. No model API or harness is hard-coded. Missing, invalid or failed output never counts as a fresh summary; cadence and call limits bound automatic calls. Summary failure or budget exhaustion does not stop deterministic monitoring, work or alter workflow state.

### LE-123: Software-change driver guidance exposes an explicit human/driver-owned Git checkpoint decision at the existing pre-review boundary.
- Status: live
- Coverage: e2e/journey

After implementation triage, the driver seeks owner authorization, inspects the staged surface and verifies any resulting commit before review. Workers do not commit independently; no new workflow state or automatic engine/provider Git lifecycle is introduced. Pending or declined authorization is not a commit.

### LE-124: A separate post-commit delivery pointer can connect reviewed run/tree evidence to matching landed content and observed hosted outcomes without rewriting terminal evidence.
- Status: live
- Coverage: e2e/journey

The driver-owned pointer refuses content mismatches, preserves pending versus observed Git/hosted facts and does not authorize delivery or imply semantic approval. Existing terminal reports and run records remain immutable.

### LE-125: Bookends continuity preserves full/shallow-clone parity and never treats unavailable required parent history as first adoption.
- Status: live
- Coverage: e2e/journey

Live-ID disappearance, tombstone removal, reassignment and revival are refused in both full and CI-equivalent shallow clones. The gate obtains required parent history or fails closed, while genuine first adoption remains valid; publication checks cover the introduced history required by LE-133.

### LE-126: Bookends bypass permission requires durable local invocation evidence and remains distinct from GREEN.
- Status: live
- Coverage: e2e/journey

Retain invocation time, repository/revision, bypass class/reason and outcome locally. Recording failure refuses bypass permission to push; normal GREEN/RED behavior is preserved and runtime evidence is not committed.

### LE-127: Bound model-worker context remains compact and usable without losing meaningful records or weakening durable evidence verification.
- Status: live
- Coverage: e2e/journey

Repeated engine-owned binding, preamble and schema evidence does not overwhelm meaningful bound model-worker context. Preserve original meaningful context IDs, judgments and ordering, current assignments and necessary instructions, full durable history and engine/provider verification evidence. Deterministic and other non-model consumers retain necessary data. Actual bound fan-out proof demonstrates compact delivered input and genuine evidence verification, including invalid-evidence refusal; separate plan-graph regression preserves correct inputs, output and evidence handling.

### LE-128: Context-only appends preserve an in-flight evaluation's snapshot and outcome.
- Status: live
- Coverage: e2e/journey

Appending context while a checked evaluation runs does not by itself invalidate its allow or deny. That evaluation uses its original input snapshot. Later evaluations receive the appended records in durable order. A change to the source-state visit or lifecycle still invalidates an evaluation based on the old state. This replaces LE-38's exclusion from the guarantee.

### LE-129: Bound execution preserves actual worker outcomes, captured output and owned-work cleanup barriers.
- Status: live
- Coverage: e2e/journey

A completed invocation reports the actual bound command's exit outcome and retains available output and inner-worker results. Process success does not establish output conformance or semantic acceptance. Waiter loss without a durable terminal result is reported as failed or incomplete, and never proves that owned work has stopped. Surviving owned work and pending cleanup block retry and departure. Helper arrangement and direct parentage are implementation choices. The execution support remains invocation-scoped and requires no persistent daemon. This replaces LE-79; the historical topology mismatch remains part of the old audit.

### LE-130: Software-change rigor levels enforce their declared review coverage and stages.
- Status: live
- Coverage: e2e/journey

All shipped levels include ordinary and challenge review at intent, design, plan, implementation and validation, using the full axis sets specified by the software-change PRD. Minimal requires one independent author covering every axis together. Standard requires two independent authors, each covering every axis together.

High requires two independent authors each to review every axis individually, followed by those same two identities each reviewing all axes together on the unchanged subject before fixes. First aggregate reviews use fresh sessions and receive neither author's individual-stage findings or judgments. Both stages remain visible and separately required. After fixes, only affected individual axes need fresh review; unaffected coverage requires explicit applicability. Both aggregate authors then freshly review every axis.

Every assigned axis appears exactly once in its commissioned output. Mixed pass/fail output is valid. Missing, duplicate or unknown axes cannot satisfy review. The existing one same-worker conformance correction preserves both raw attempts. Candidate projection retains the real invocation/assignment origin and labels carried references separately. Exact-source dispositions remain subject to LE-115; accepted unresolved findings block across revisions. No stage can satisfy the other stage's obligation.

Final criterion and whole-goal author counts are one for minimal and two for standard/high, independently of axis batching. High's extra stage creates no additional author floor or proof-suite execution. Shipped profiles remain unbound and contain no selected model or machine-local command. Bookends remains off until explicitly enabled for a run. This replaces LE-118's universal single-commission default.

### LE-131: Each software-change implementation task names the current acceptance criteria it serves.
- Status: live
- Coverage: e2e/journey

New supporting profiles require a nonempty set of current criterion references on every implementation task. Missing, duplicate or unknown references fail deterministic validation. Reviewers judge whether the links meaningfully cover the task's work; unrelated links cannot establish coverage. Other intermediate links remain optional. Final validation still covers every current criterion and the whole goal. Task references introduce no second requirement-ID system.

### LE-132: A small explicit worker roster produces inspectable per-run software-change setup.
- Status: live
- Coverage: e2e/journey

An operator selects a rigor level and supplies worker commands and arguments, including model and effort arguments where applicable. A deterministic setup utility assembles the matching profile, independent author assignments, review stages and optional implementation binding from shipped data. Any command meeting the worker input/output contract is eligible. Invalid, incomplete or insufficient input fails without reducing review obligations.

The resulting policy and commands are visible for owner confirmation before start. Setup does not start a run, select models, adapt incompatible worker CLIs, save implicit preferences or manage worktrees. Shipped data works through both source and packaged interfaces. Semantic provider operations retain their existing work and judgment boundary.

### LE-133: Bookends checks every newly published reachable requirement transition.
- Status: live
- Coverage: e2e/journey

Pre-push and required CI inspect every commit introduced by each updated ref, including merged branch commits, and check continuity against each relevant parent. A later repair cannot conceal an invalid intermediate transition. Legitimate first adoption and permanent tombstones retain their meaning. Coverage of the published tip is checked against that tip's content.

Local and hosted checks obtain required history before claiming complete coverage. Equivalent available history produces the same decision in full and shallow clones. Missing history, incomplete enumeration, interruption or a resource limit cannot produce complete GREEN. A new ref cannot use a guessed baseline to omit its ancestry. Explicit bypass retains its reason and durable invocation evidence under LE-126.

### LE-134: Bookends coverage uses approved public proof locations and observable assertions.
- Status: live
- Coverage: e2e/journey

Only configured, approved proof locations with established collection can supply coverage. Documentation examples, incidental strings, generated/vendor content and internal tests cannot accidentally fill a public-proof gap. Fixture examples and skip directives keep their declared meaning.

A genuine public API or CLI contract test is eligible when its observable assertions prove the requirement. A broader journey obligation still requires proof of that broader outcome. Mechanical citation and collection checks do not establish semantic sufficiency; independent review inspects the actual assertions and retained results. Applicable existing public proof can be reused without creating duplicate tests merely to change its label.

### LE-135: Default run storage and work locations remain discoverable across working directories.
- Status: live
- Coverage: e2e/journey

Without an explicit storage override, ordinary operations use a consistent user catalog across working directories. A new run receives its own durable work location by default, and inspection exposes that location for a fresh actor. Independent runs do not require caller-created isolation to avoid clobbering those locations. Explicit catalog or artifact-location choices remain supported. Path spelling and environment-variable precedence belong to the CLI specification.

### LE-136: Callers can supply stable run and context-record identities and recover them unchanged.
- Status: live
- Coverage: e2e/journey

The public interface accepts caller-selected run and context-record IDs. A successfully accepted ID is preserved in the result and subsequent durable inspection, including context history. The engine may generate IDs when the caller omits them. This requirement does not introduce a new identifier grammar or migration rule.

### LE-137: Software-change semantic evaluation checks durable review evidence without doing the review.
- Status: live
- Coverage: e2e/journey

The provider's describe/evaluate interface preserves the engine-selected transition and validates configured artifacts and durable evidence. It performs no semantic judgment, reviewer launch or primary work. A separate deterministic setup utility may assemble static shipped prompts, schemas and caller-supplied commands into an inspectable execution profile. This qualifies the former unscoped prompt-formatting prohibition in LE-45 without moving review execution or judgment into semantic evaluation.

### LE-138: Generic fan-out preserves declared ordering, worker contracts and honest captures.
- Status: live
- Coverage: e2e/journey

Without an explicit barrier, workers remain parallel subject to the caller's concurrency bound. An optional barrier orders two worker groups while preserving stable assignment identities and selected execution. A conforming semantic failure may proceed to the independent later group. Execution and conformance failures remain visible at their respective boundaries. An uncontracted worker's nonzero exit remains recorded even when the facade exits zero; a declared conformance failure fails the facade. The engine interprets ordering and output shape only.

Existing compact bound framing, unchanged ad-hoc instruction bytes, declared legacy/full output contracts, true inner exits, retained attempts, mechanical join and failed-graph capture fallback remain supported. Captures are available before facade failure is reported. Process exit and conformance remain separate from semantic acceptance. No implicit workflow retry or semantic routing is introduced.

Fan-out retains its uncapped default when no concurrency bound is supplied. Plan-graph retains its default of four ordinary tasks, an explicit bound when supplied, and its summarizer after selected tasks. Exact argv, packet/framing bytes, scheduler encoding and capture fields belong to the CLI specification. This replaces LE-90's prohibition on inter-worker dependencies; it preserves the existing unbarriered behavior.

### LE-139: Public-boundary journeys prove workflow and worker contracts without live models.
- Status: live
- Coverage: e2e/journey

Public-boundary journeys (`scripts/software-change-journey.py`, `scripts/policy-document-journey.py`, `scripts/research-journey.py`) freeze a sparse dummy-worker binding, invoke before the bound checked event, and prove catalog snapshot, instruction redaction, unbound-invoke rejection, pre-evaluate gate, worker-packet stdin, overlay `succeeded`, unbound stored instructions, and invocation history. Software-change journeys also prove unbound shipped profiles, graph-runner and fan-out behavior with dummy inner workers, `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, `preview-bindings` warning when pi has `--no-extensions` and no `-e`, and do not call a live model. `scripts/software-change-journey.py --self-test` executes the software-change setup utility and two provider skill constructors against software-change high-rigor policy/stage setup, policy-document shipped semantic policies/target/mode, and research verify plus synthesize; it compares worker count/order and exact axis/`example_prompt`/author/model/subject metadata to each source profile, asserts required keys/data bytes/preview visibility and fail-closed invalid cases, asserts root AGENTS rules, and prints `worker-data skill/root policy assertions passed` only after all pass. The software-change source full journey binds deterministic stdin-capturing workers that emit conforming JSON or exit-0 refusal text and, through separate public CLI processes, asserts the compact one-key `artifact_root` context precedes the separator/body with no `capture_dir` or duplicate identity in that block, conforming `status` succeeded, refusal `status` failed with `exit_code` 0, summary/captures persist, overlay fails, then prints `contracted fan-out failure`.

### LE-140: Opt-in review bindings use the extensions their selected model providers require.
- Status: live
- Coverage: e2e/journey

Bound review slots frozen to `fan-out` still require `loop-engine invoke RUN_ID SLOT_ID`. A usable review binding contains provider-constructed assigned `--worker` objects frozen at `start` after `preview-bindings` and lock-in; a review slot with an empty configured policy-axis list is not bound. Shipped profiles omit `work_slot_bindings` so slots stay driver-performed. Opt-in skill templates keep `--no-extensions` and add an existing `-e` path for each extension required by the selected model provider; omitted paths produce no `-e` pair. Default implement inner argv when `--task-worker` is omitted remains `pi --print --no-skills --no-extensions` and must not pass `--no-context-files`. A supplied extension path must be absolute and exist.

## 15. Complexity Guardrails

The original v0.1 design targets were:

```text
provider semantic operations: 2
primary caller operations:    10 (proposed recovery amendment)
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

A new core concept or subsystem must solve a demonstrated limitation of a reference workflow or real integration. Simple-first, YAGNI and KISS apply product-wide under the proposed LE-107 amendment; existing mechanisms have no presumption of preservation.

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

## 17. Historical v0.1 deferrals

The following capabilities were deferred during the initial design. Later accepted requirements supersede this historical list; the list itself authorizes no additional features:

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
