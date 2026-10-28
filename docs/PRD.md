# Loop Engine v2 — Product Requirements Document

**Status:** Living
**Target:** Current 0.x product; dated v0.1 scope is retained as history
**Compatibility:** Clean-slate successor; no v1 compatibility requirement
**Amended:** 2026-09-11

> **Recovery amendment draft (plan r5): owner acceptance and commit pending.** The recovery wording integrated below and the explicitly marked requirement amendments are proposals against the implemented contract, not a claim of accepted policy or completed audits. Existing titles and amendment history are retained; LE-51 remains tombstoned. Historical runs retain their original obligations. The owner must accept the exact diff; the driver completes document audits before final proof.

> **Backlog amendment scope:** The historical-scope labels and changes to execution description, context-only evaluation, LE-100/102/110/116/119/125, replacements LE-128/129/130/137/138 and additions LE-131–136 are one scoped amendment. They do not accept unrelated recovery-r5 proposals or reinterpret historical runs. LE-135 and LE-136 backfill intended behavior already present in the current product.

## Current external-work successor scope

The external-work amendment is limited to LE-167–LE-183 and the explicit qualifications and tombstones below. It does not accept unrelated marked recovery proposals, the pending LE-107 recovery paragraph or software-change provider Section 10. Original dated history remains history. Existing runs and data are not migrated; availability of historical binaries or detailed execution projections is not required.

For the supported external-only product, the engine owns durable progression and callers own all agents, advice and proof/test commands. Deterministic provider describe/evaluate and necessary metadata checks remain. The seven primary engine operations are start, list, show, append, event, history and terminate. Separate deterministic provider preparation/recording/inspection is not a work executor. Earlier narrative about retired bindings, invoke/waiters, fan-out/plan runners, capture commands, engine summaries and primary execution controls describes historical behavior, not a parallel supported path. Earlier absolute floor/eligibility/freshness language is qualified only by the named new records; immutable initial input and all other semantic policy remain truthful.

These records govern applicable behavior of policy-document and research, and governed software-change while it shipped; lean-change is governed by LE-202–LE-209. Provider-specific target/phase meaning remains. Policy-document gains explicit configured groups/counts/eligibility and same-run applicable evidence rather than relying on its former fixed one-pass/no-floor rule; its actual target authorship is a declaration, not a Git-committer inference or registry. Research's adversarial question remains contrary-evidence checking, not an invented software-change phase. Actual requests/returns can be prose/YAML/JSON; strict normalized projection belongs to supported driver recording, not a mandatory reviewer output schema.

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

A **check-free** transition does not invoke provider evaluation. For a new run that explicitly enables the advisory departure contract in LE-166, its normal departure also requires advice closure or a scoped owner-attested advice exception; observation and owned-work cleanup remain required. Older or advisory-disabled graphs acquire no such prerequisite.

A **checked** transition normally requires provider `allow` before it may commit. Opted-in driver completion under LE-153 only replaces the narrow pre-provider bound-worker-success check; it does not replace provider `allow`. Explicit owner-attested override is a distinct exceptional transition outcome, never a provider allow.

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
3. if the checked edge is a **bound** work slot (present in frozen `work_slot_bindings`), normally requires an overlay-`succeeded` invocation matching slot ID, `instruction_digest` (SHA-256 of the stored instruction body UTF-8 bytes, lowercase hex), and the current **slot-visit** subject from get-current-subject. Only a new graph explicitly opting its non-review draft/implement slot into LE-153 can use a distinct driver-attributed completion act instead of that invocation; an act is not a worker success. Overlay `running`, `failed`, or `overrun` alone do not allow the normal edge. All departures first require quiescent owned work; check-free edges omit provider and bound-success checks. For a run explicitly enabling LE-166, checked and check-free normal departures also require due advice and successful-answer dispositions, or a scoped owner-attested advice exception for unanswered due occasions; this is not provider evaluation or a work-action interlock. An explicit general override remains a separate exceptional path after quiescence;
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

### 6.8 Historical invocation (retired from the supported product)

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

Plan-graph execution uses the driver's existing repository, preserves dependencies and standing prerequisites, and writes a current report/checkpoint after successful selected work. The existing no-task repair path remains limited to an accepted implementation finding with no honest frozen task owner. For explicitly supporting future software-change graphs, LE-150 permits verified same-run standing mappings, and LE-153 permits a separate report-only same-bound-graph summarizer selection after a checked reconciliation decision when every task stands. Omitted input still runs the full plan; task roots remain nonempty; no-task repair still runs a different worker without the summarizer. None of these paths manages worktrees or chooses semantic recovery routes. Existing frozen runs retain their original runtime, input and available selectors.

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

## 10. Reference Workflow A — Lean Change

The software-change provider and its fifteen-state phase/review topology are retired. The lean-change provider replaces it as the software reference workflow; its requirements are LE-202–LE-209 in §19. The tombstoned software-change requirements in §14.5 and §14.9–14.10 keep their IDs and titles for continuity and historical interpretation.

Runs started under software-change keep their stored graphs, context and history. A newer engine still lists and shows them without the provider, and their check-free edges remain usable from the stored graph when the provider is unavailable (LE-54). Nothing is migrated.

The provider may inspect repository state, documents, tests, reviews, or other software-specific information. Core understands none of those concepts.

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

LE-166 adds a frozen opt-in advice-closure prerequisite to normal departure on new enabled runs, not provider evaluation or a new action gate. Disabled/older runs retain the original behavior; cancellation and cleanup cannot wait on advice.

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

**Engine-owned advice qualification:** The engine's explicit advice-policy and occasion-closure rules under LE-166/189/192 select the effective map and latest linked occasion declaration without rewriting earlier context. This narrow engine-owned bookkeeping does not assign provider evidence truth, review approval or general provider-record supersession meaning.

### LE-19: Every checked evaluation receives all accumulated context records in durable append order.
- Status: live
- Coverage: e2e/journey

### LE-20: `show` gives a fresh actor enough information to resume without the previous conversation or raw history, assuming access to externally referenced work.
- Status: live
- Coverage: e2e/journey

Ordinary `show` provides current action instructions; separate status-only and full inspection views remain available. Status-only observation does not arm mutation. Action and full instruction reads arm the current visit. Optional provider-authored action guidance remains opaque to core; absent legacy guidance means unknown normalized obligations, not zero obligations, and preserves the existing bound-invocation or external-work path and access to frozen policy. Full inspection retains complete configuration, context and invocation/change-report evidence with original identities and ordering. The focused action view exposes current obligations, active work, source-located blockers with explicit freshness or uncertainty, and references for omitted material; unrelated completed invocations and historical context do not enlarge it.


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

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


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

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

### 14.5 Software-Change Workflow (retired)

The software-change provider was replaced by lean-change (§10). These records are retained as tombstones.


### LE-39: A minimal idea can be driven to completion.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-40: Substantial already-known intent/design/planning context can use the same workflow.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-41: A run can configure semantic review policies for the review/validation gates in immutable initial input.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-42: The configured policies are available to a fresh actor through `show` without a provider call or separate policy-discovery operation.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-43: The same workflow topology and software-change provider mechanism can execute runs with materially different review-policy configurations.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-44: Human or agent reviewers can perform each configured semantic review externally and append the resulting pass/fail findings as ordinary durable context using the software-change provider's workflow-specific evidence convention.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-45: The reference software-change provider does not itself perform the semantic review or generate formatted review prompts; it validates whether the configured review obligations have acceptable durable evidence before allowing progression from a review/validation gate.
- Status: tombstone

Superseded by LE-137. The original title is retained for continuity and historical interpretation.

### LE-46: Missing or failed required review evidence denies the corresponding checked approval with actionable feedback identifying the unsatisfied policy obligation.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-47: Acceptable evidence for all configured policies at a gate allows that policy portion of the gate to pass without introducing engine-level policy or review-result semantics.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-48: At least one review denial and revision cycle is demonstrated.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-49: After a review denial and check-free revision edge, a fresh actor can see the configured policies, durable review evidence, and actionable review feedback through `show` without reading `history`.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-50: After a later successful evaluation, the previous denial is no longer projected as the latest result for that transition.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-51: User steering can affect later work and evaluation.
- Status: tombstone

### LE-52: Prior review evidence and evaluation lineage can inform later review/check cycles.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-53: A run can move between distinct actor sessions or harnesses.
- Status: live
- Coverage: e2e/journey

### LE-54: A check-free revision edge remains usable when provider evaluation is unavailable.
- Status: live
- Coverage: e2e/journey

For an LE-166-enabled run, provider unavailability is still not a check-free gate, but a normal revision needs its own due advice closure or scoped owner-attested advice exception. An unavailable advisor is not silently treated as an answer; cleanup/cancellation remain possible.

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


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

### LE-65: Topology covers scope, gather, adversarial verify, and synthesize.
- Status: live
- Coverage: e2e/journey

### LE-66: Checked transitions refuse until artifacts satisfy declared structure and independent evidence satisfies declared review obligations at verify and synthesize.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-168/171 qualify unconditional subject-author exclusion, singleton commissioning and fresh-only revision handling for explicitly configured eligible self-review, groups/stages and same-run applicability. Research keeps its scope/gather/verify/synthesize and contrary-evidence meaning; it gains no software-change phase or automatic cleared-parent challenge interpretation.

### LE-67: Local blackbox tests exercise at least one checked denial and a successful completion.
- Status: live
- Coverage: e2e/journey

### LE-68: CI preflight builds the research binary and runs a source journey; archive-smoke runs a packaged journey after materializing embedded data.
- Status: live
- Coverage: e2e/journey

**Lean CI/release amendment:** The exact heading above is retained solely as historical requirement identity text. It does not require the retired pipeline; this body supersedes the heading's former operative pipeline description. Blocking CI in `ci.yml` runs only on pushes to `main` and requires formatting checks, all-target workspace Clippy with `-D warnings`, a locked workspace build, workspace tests, the bare self-tests of all five journey scripts (external-work, software-change, policy-document, research and generate-PRD), and the Bookends gate over every introduced reachable commit from main's tip before the push (`before`) through the pushed tip (`after`), not merely the pushed tip against its own parent. Bookends retains the history and continuity obligations of LE-125 and LE-133. A separate informational workflow runs the full source journeys on the same main pushes and exposes their pass or fail; it never gates Release, including when failed or still running.

### LE-69: cargo-dist plan and release-gate assertions include the research binary.
- Status: live
- Coverage: e2e/journey

**Lean CI/release amendment:** Release is dispatch-only, with no push or pull-request trigger. Every dispatch, including dry-run, requires a prior successful push-event `ci.yml` run for the exact dispatched commit before artifact compilation; absent or unsuccessful exact-commit proof stops the already dispatched workflow before artifact compilation. On every dispatch, every executable in every built native release archive for macOS arm64 (`aarch64-apple-darwin`) and Linux x86_64 (`x86_64-unknown-linux-gnu`) must exit 0 for `--version` and report exactly one whole version token equal to that archive's release version, not a substring or prefix match. Publication is admitted only when archive-check succeeds; failed, skipped, cancelled or missing archive-check results cannot admit publication. The research binary remains included in cargo-dist plan and release-gate assertions and in the native release matrix for both targets.

### LE-70: The provider does not fetch, invoke models, or judge semantic truth.
- Status: live
- Coverage: e2e/journey

### 14.8 Operational Simplicity

### LE-71: Local operation requires no daemon or external infrastructure beyond Loop Engine's local durable state and configured provider integration. Hidden `wait-invocation` is a short-lived per-invocation waiter, not a background service.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

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
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-76: Omitted `work_slot_bindings` and `{}` both mean no bindings; unknown slot IDs, unknown binding fields, and non-object values are rejected at `start`. `start` does not parse `fan-out` or `run-plan-graph` argv. `preview-bindings` exits nonzero on a zero-worker `fan-out` freeze.
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-77: When a slot is bound, `current_state_instructions` names the slot ID plus the frozen CLI binding `{command, args}` and that the legal start is `loop-engine invoke RUN_ID SLOT_ID`; it omits the stored work body and states the bound-instruction triage order (overlay succeeded is bound CLI exit 0, not provider acceptance; captures are at the named directory; the driver triages, appends, then requests the shown event; on overrun run `show` immediately before re-invoking; on failed inspect `summary.json` and captured output before stderr).
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-78: `invoke` is the only legal start for bound work. On accept it allocates `capture_dir` as `{artifact_root}/work-slot-captures/{slot_id}/{invocation_id}`, creates that directory, stores it, and returns it. The bound worker's stdin is exactly one JSON object with `run_id`, `slot_id`, `artifact_root`, `instruction_body`, and `capture_dir`, plus optional `context` when the slot declared nonempty `stdin_context_kinds` (not argv, environment, or a temp file). Waiter stdin is not the worker packet.
- Status: tombstone

For expressly opted-in new non-review draft/implement slots, LE-153 permits a separately recorded driver-authored completion **instead of** worker success at the pre-provider gate; it is not a bound worker start, invocation or reviewer act. Review slots remain invoke-only. The normal checked provider gate still runs. Future invoke packets may include separately identified engine-selected assignment/opaque input, generic labels and transition history needed to verify LE-150/153; they are not driver-authored provenance or a change to the frozen older worker packet.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-79: Hidden `wait-invocation` is parent of the bound worker, waitpids it, writes terminal `succeeded`/`failed` plus `exit_code`, then exits. After waitpid, a well-formed `capture_dir/summary.json` is copied as `inner_workers` (`command`, `args`, `exit_code` only); overlay remains the bound CLI process exit. It is not a daemon. A vanished waiter with no terminal status is overlay-`failed`.
- Status: tombstone

Superseded by LE-129. The original title is retained for continuity and historical interpretation.

### LE-80: Hidden `stdin-exec` opens a stdin file, attaches it to child stdin, and runs `COMMAND [ARG]...` after `--` with no shell. Duty bytes stay in that file (not argv or environment). Sidecar mode writes `{"exit_code": <inner waitpid as i32>}` then exits 0; propagate mode is the inner waitpid and rejects `--sidecar-file`. Spawn failure exits nonzero without a successful sidecar. `--help` omits it. When `PI_CODING_AGENT_SESSION_DIR` is unset in the inherited environment, stdin-exec colocates Pi sessions under the worker `capture_dir/sessions` via that variable at spawn; frozen argv does not add `--session-dir`. `software-change` duplicates the same helper; plan-graph uses propagate mode only.
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-81: Invocation records are engine-authored; `append` cannot write them. History records `invocation started {invocation_id}` and `invocation status changed {invocation_id, status}` for waiter-written `succeeded`/`failed` only.
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-82: `work_slot_invocations.status` is the reader overlay `running` | `succeeded` | `failed` | `overrun`. Each view also reports `overlay_meaning`, `elapsed_ms`, `remaining_allowed_ms`, `capture_dir`, and `inner_workers`. Overlay `overrun` is not a history action. `waiter_pid` is not in `show`. `show` does not spawn a provider and does not read capture files. While overlay is `running`, the canonical driver poll is `show` for overlay (`inner_workers` empty) plus `invocation-progress` for inner graph/traces. Graph state is Dagu helper liveness (`reaped` is helper finished, not overlay success and not inner waitpid 0). `dagu status` / `dagu history` remain the underlying surface `invocation-progress` uses, not the driver-facing path.
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-83: A bound checked edge is refused unless overlay status is `succeeded` matching slot ID, `instruction_digest`, and the current slot-visit subject. Overlay `running`, `failed`, and `overrun` do not satisfy. Check-free edges are ungated. `evaluate` never waits.
- Status: tombstone

LE-153 supplies only a narrow future non-review driver act as an alternative to the matching succeeded-invocation pre-provider check, not to provider `allow`. For new LE-166-enabled runs, normal checked and check-free departures also require due advice closure or a scoped advice exception; check-free still omits bound-success/provider checks. Older/disabled runs retain their rules.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-84: Overlay `overrun` is terminal for retry: a later `invoke` of the same slot is not already-running, but the driver runs `show` immediately before re-invoking. Failed and overrun records remain inspectable and never count as success. On failure the driver inspects `summary.json` and captured output before stderr. Overlay succeeded remains the bound CLI exiting 0 even when stored `inner_workers` contain a nonzero `exit_code`.
- Status: tombstone

> **Proposed retirement, recovery r5:** elapsed allowance cannot authorize overlapping live work. LE-110 replaces retry permission; history and capture inspection remain required.

### LE-85: When a slot has no binding, the driver may perform that job and no invocation record is required. When the binding set is empty, a run can still complete with the driver performing the work.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

### LE-86: Policy-document has no work slot for `prepare` → `ready`. Software-change, policy-document, and research share the same binding, invoke, overlay, and gate contract; each only declares its catalog.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

### LE-87: Slot-visit subjects are minted via set-current-subject on entry into a slot state, including `start` when the initial state is a slot. `invoke` snapshots via get-current-subject and does not mint. `instruction_digest` is SHA-256 of the stored instruction body UTF-8 bytes, lowercase hex.
- Status: tombstone


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

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
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-92: Reviewer output is candidate evidence only. The driver owns the append-only current `finding-ledger` disposition and routing view: an advisory classification proposal is inert until accepted or edited, and only current accepted unresolved implementation findings explicitly routed to a task or review axis affect packets or later gates; raw candidate sources and driver decisions remain distinguishable.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-93: A bound reviewer using `full_output_schema` receives at most one correction opportunity under the identical frozen command, arguments, assignment, preamble, and model. The first and second raw stdout/stderr attempts, exact validation errors, selected attempt, and exhaustion state remain retrievable; a second invalid response fails closed without a substitute reviewer or semantic verdict.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-94: `implementation-ready` cannot advance from an author-declared implementation report alone. The provider-generated implementation checkpoint independently binds the report and document revisions to the current repository HEAD, index, status, tracked/non-ignored-untracked entries, and content identity, and the public checkpoint command is read-only with respect to Git.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-95: After implementation or final validation proof is accepted, changing repository HEAD, adding or deleting a tracked or non-ignored untracked entry, renaming an entry, changing status or type, or changing tracked bytes makes the affected checkpoint stale and refuses later checked progression until proof is regenerated. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, whether or not implementation review is configured. Validation requires the sole accepted entry for the current report revision to match the current report, document revisions, and repository state. Appending later context, overwriting history bytes, or replacing both mutable checkpoint files cannot admit bytes that transition did not accept.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-96: When validation exposes a stale repository checkpoint, the validation draft and review states expose a check-free `revise-implementation` recovery route. Final approval requires regenerated implementation and validation checkpoints for the same current tree plus passing review evidence; validation cannot silently replace implementation proof.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-97: Plan and validation review require observable user or operator outcomes, pragmatic black-box proof or a concrete impracticality reason, and semantic inspection of every new or changed Bookends citation. A final validation report maps requirements to observable proof, and passing activity or a matching requirement token alone is not treated as completion.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-98: `show` of current state and instructions is the observation that arms mutation. `append`, `event`, `invoke`, and `terminate` refuse without a current observation, preserve state and semantic history on refusal, and require a new observation after every state visit, including a self-loop; `list`, `history`, and `invocation-progress` do not arm mutation.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

### LE-99: A completed bound invocation durably identifies each enumerable assignment independently, including selected attempt or coverage gap, the digest of selected originating bytes, and their originating-attempt location. A worker-level copy is not the selected-attempt identity, and these facts remain inert until a driver acts on them.
- Status: tombstone

Retired with the software-change provider; bound-invocation selection was already retired (LE-176). The original title is retained for continuity and historical interpretation.

### LE-100: Ordinary `show` exposes a deterministic provider-free, fail-closed change report for assignment records and recorded plan-task results. It reports covered subject, assignment/binding, policy/configuration, output-contract, routed-input, task-definition/packet, dependency, worker-binding, and task-recorded repository-effect dimensions; unknown inputs are changed; standing records and results are visible from the durable run without provider execution or capture-file reads.
- Status: live
- Coverage: e2e/journey

`show` exposes a deterministic provider-free, fail-closed change report for assignment records and recorded plan-task results. Full inspection reports covered subject, assignment/binding, policy/configuration, output-contract, routed-input, task-definition/packet, dependency, worker-binding, and task-recorded repository-effect dimensions; unknown inputs are changed; standing records and results are visible from the durable run without provider execution or capture-file reads. Ordinary action inspection exposes selected relevant change-report facts and references to full inspection rather than unrelated historical reports.

For two otherwise present task records, an optional repository effect omitted by both is a known equal absence. A missing required record or dimension remains unknown and changed. This distinction cannot revive a result replaced by a later real failed execution.


**External-work successor qualification:** LE-170/177 replace new evidence dependence on same-worker retry, engine invocation/assignment selection and captured packets with actual supplied requests/instructions, original returns and faithful separately attributed driver-confirmed projection. Preserve known identity/content checks, original result/author/findings/rationale, visible gaps and inert candidate previews. Valid siblings survive malformed siblings; missing substantive meaning is not formatting. No engine-executed claim or new work registry is created.

### LE-101: `invoke` may select only named enumerable assignments using the existing invoke path. Empty, duplicate, unknown, or non-enumerable selections refuse before a worker starts; argv that resembles fan-out behind an executable other than the current engine remains non-enumerable; omitted selection runs the frozen binding in full; the validated selection is durable and never rewrites the frozen binding.
- Status: tombstone

Here, selection means engine-owned `--assignment`/`--assignments` selection. A bound invocation may additionally accept one distinct optional opaque JSON value through `invoke --input`; core validates only JSON framing, rejects `--input` together with assignment selection, durably stores and transports the exact value, preserves the frozen command and args, and does not interpret provider semantics. Omitted input remains full execution.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-102: `software-change run-plan-graph` may select plan-task roots plus their dependants without auto-including missing prerequisites. Invalid selections refuse before Dagu or a task starts; omitted selection remains full execution; every successful invocation still runs the summarizer and repository checkpoint against the resulting working tree, including effects left by unselected tasks.
- Status: tombstone

Direct `--task`/`--tasks` selection and bound software-change implement invocations may use their respective selection forms; the existing bound path interprets `invocation_input` exactly as `{plan_revision,task_roots}`. Explicit future LE-150 standing-result mapping may compare original results with revised same-run obligations across plan revisions, but cannot claim standing from names or file equality. Under LE-153 a separate future `{plan_revision,report_only:true}` selection, after checked LE-142 reconciliation and re-entry, verifies all current tasks standing and runs only the same frozen graph summarizer for a fresh report/checkpoint; no task is asserted newly performed. A frozen graph without these selectors has no implied report-only route; use supported selection only for genuine affected task work, otherwise stop and escalate. Existing task-root selection still rejects malformed, empty, duplicate, unknown, stale-plan, missing-prerequisite or ambiguous roots before Dagu, any plan-task worker, summarizer or checkpoint. Roots include transitive dependants; each unselected prerequisite must stand for the current plan revision, either from a same-revision original result or from an explicitly verified LE-150 source-to-current-obligation mapping in a supporting new graph. Report-only requires all current tasks standing and runs no plan task; it cannot be expressed as empty task roots. Omitted direct selection and omitted input remain full execution.

An invalid retry refused before any task starts must preserve previously standing prerequisite results. Successful selected recovery reuses only currently applicable prerequisites and retains the required summarizer/report/checkpoint outcome.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-103: `unchanged-carry` on the existing append path consults the durable change report and refuses when any covered input changed. A successful carry preserves the originating author's identity and selected-output digest, records the attesting driver and carry act separately, and makes the result distinguishable from a fresh worker judgment. It attests the exact report snapshot it saw; later drift makes the contribution non-standing until another explicit act.
- Status: tombstone

> **Non-normative historical note:** Completed runs may expose `unchanged-carry` records from the former contract through immutable inspection. New appends use `evidence-applicability`; no migration or compatibility path is provided.

### LE-104: `override-carry` on the existing append path is distinct from `unchanged-carry` and requires the driver to name every changed covered input. The durable run exposes the act and overridden inputs; the engine records the attestation but does not decide whether the carry was warranted.
- Status: tombstone

> **Non-normative historical note:** Completed runs may expose `override-carry` records from the former contract through immutable inspection. New appends use `evidence-applicability`; no migration or compatibility path is provided.

### LE-105: A software-change evidence record linked to selected originating bytes is accepted only when its invocation, assignment, selected attempt, digest, and path match the engine-selected durable record and its availability and mechanical judgment fields agree with those bytes. Fabricated identity is refused at append; missing, changed, unavailable, or disagreeing bytes are unverified and cannot satisfy a checked transition; worker invocation records alone remain inert.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-106: Released provider binaries retain sufficient embedded data for `data-dump`, shipped profiles, templates, and reviewer protocol, and a described/evaluated run can use that data without a checkout at runtime.
- Status: live
- Coverage: e2e/journey

### LE-107: Minimum provenance is a paramount product constraint.
- Status: live
- Coverage: e2e/journey

> **Amendment history — this proposal was accepted through owner-delegated artifact review on 2026-08-26; its authority is established only by a separate commit.** For Loop Engine and the software-change provider, assume a trusted sole owner and well-intentioned, instruction-following drivers. Provenance serves only fresh resume, focused invalidation, honest bypass, debugging, and visible history. Capture each fact once under a stable engine-owned identity and reference that origin; keep ordinary driver-authored metadata small. Trust explicit driver declarations of materiality and evidence applicability except for cheap mechanical identity mismatches. Prefer the narrowest honest correction, preserve valid work, and do not replay still-valid work merely for ceremony. Actual failures, findings, overrides, evidence, checkpoints, and terminal history remain visible. Every proposed provenance field, identity dimension, gate, state, or replay rule must identify the observed ordinary-use failure it prevents and explain why existing durable state, history, capture, or driver judgment is insufficient. This preserves LE-18 and the engine/caller/provider ownership boundary: core assigns context no truth or provenance, callers perform work externally, and providers evaluate the exact selected transition without routing or judging semantic truth. It adds no engine truth model, gate, state, replay system, or provenance framework.

> **Proposed LE-107 amendment, recovery r5 — owner acceptance pending:** Simple-first, YAGNI and KISS govern Loop Engine as a whole and every provider it generates, ships or uses. Start with the simplest adequate solution. Added complexity requires a documented meaningful current requirement or observed ordinary-use failure and a brief explanation, in the ordinary design, of why an adequate simpler approach is insufficient. Gold-plating, productionization for its own sake, speculative defense-in-depth and adversarial guards without that basis are not wanted. Current implementation, architecture, communication protocols, schemas, dependencies and internal mechanisms have no presumption of preservation. A simpler replacement or removal is in scope when it meaningfully reduces complexity, scope, footguns or ordinary-use failures; change for novelty and compatibility scaffolding solely to preserve incidental design are not justified. Keep driver-authored provenance small and capture mechanical facts once, while retaining honest failures, findings, exceptions and rich engine history. Trust explicit materiality/applicability declarations except cheap identity mismatches. Review judges the ordinary design and delivered outcomes; no separate justification framework, gate or metadata inventory is required. Policy-document and research gain common simplicity/execution guidance, not software-change-specific finding or criterion features.

### LE-108: A bound software-change implementation can capture one focused no-task repair without replaying valid plan work.
- Status: tombstone

> **Amendment history — accepted for Package 5 through owner-delegated intent, design, and plan review on 2026-08-28.** The observed failure is an accepted current implementation finding for which no frozen plan task honestly owns the correction: before this amendment the driver had to revise an otherwise-correct plan, replay unrelated work, make an uncaptured off-engine correction, or strand the run. On the existing bound `implement` slot, opaque invocation input may therefore be exactly `{"repair_finding_ids":[...]}`, disjoint from omitted full execution and exact `{"plan_revision":"...","task_roots":[...]}` selection. The provider accepts only unique named findings from current engine-forwarded ledger snapshots that are accepted, unresolved, implementation-owned, current for the subject and verified implementation checkpoint, and carry empty `task_ids`; malformed, empty, unknown, stale, wrong-owner/status/disposition, or task-routed requests refuse before Dagu resolution, proof deletion, worker launch, or repository mutation. A valid request uses the unchanged frozen worker and checkout for exactly one captured `ad-hoc-repair` assignment, with the selected finding objects and provider-derived pre-repair proof identity; it runs no plan task or summarizer and does not alter `plan-task-results.json`. The worker must write a schema-valid implementation report linked to the frozen plan whose report revision is unused by both the immediately preceding proof and every accepted implementation-proof-history entry. Only then does the provider create a new implementation checkpoint. The ordinary invocation and summary retain the exact input, frozen binding, selected output, routed findings, and provider-derived pre/post report and repository-state identities; process success is not semantic acceptance, and existing independent implementation review, validation, and terminal gates still apply. Task-owned defects use LE-102 selection, materially wrong decomposition revises the plan, and no direct repair flag, core finding semantics, automatic task-fit judgment, rollback, carry redesign, binding correction, generalized replay, criterion redesign, or progress/status redesign is added.

LE-153 separately qualifies the driver-authored correction and same-graph report-only finalization **only** for supporting opted-in graphs; these are not the dated Package 5 no-task repair mode or permission for arbitrary unbound repair. Graphs without a supported route retain their original limits.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-109: A completed software-change `show` can be piped to `software-change review-candidates` for a deterministic, provider-owned view of bound review assignments. The view exposes only normalized selected-output judgment fields with a stable invocation/assignment origin, or a mechanical `malformed`, `unavailable`, `missing-selection`, or `exhausted` diagnostic. It does not retry, deduplicate across durable invocations, mutate captures or run state, append evidence, or satisfy a gate; after inspecting and triaging it, the driver must explicitly accept, edit, or reject the candidate and use the ordinary review-evidence and finding-ledger append path before requesting the checked event.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### 14.10 Proposed recovery amendments (owner acceptance pending)

The following exact qualifications amend existing live records without renaming their titles: LE-6 and LE-83 describe **normal** checked progression; LE-113 adds the explicit exceptional mode. LE-5 and LE-54 remain provider-free but subject to LE-110's live-work barrier. LE-22 visibility gains the delivered recipient selection in LE-114, not revival of LE-51. LE-46's failed evidence blocks when not discharged by LE-115; a discharged fail is not a pass. LE-75–LE-78 and LE-101 retain initial settings but expose/snapshot effective future settings under LE-111. LE-79 and LE-81 describe ordinary waiter completion; cancellation finalization belongs to LE-112 and missing waiter liveness is not cleanup proof. LE-82 retains its overlay vocabulary, never retry authority. Its no-capture-read clause is qualified: provider-free `show` reads engine-owned ownership/cancellation metadata under `capture_dir`; worker-output interpretation remains the driver's duty. LE-88–LE-90 retain transport and distinct axis obligations while LE-118 changes software-change's default constructor allocation. LE-92 uses exact-source dispositions, not text-set equality. LE-95–LE-97 require normal accepted implementation-proof history and LE-116's final index; override does not fabricate them. LE-98 observation also arms amend-binding and cancellation admission; an outstanding cancellation can be resumed. LE-109 expands grouped selected output into inert per-axis and criterion/goal candidates. Older amendment scope exclusions describe those earlier changes, not a prohibition of this explicit amendment.

### LE-110: Live invocation-owned work blocks retry and state departure until completion or verified cleanup.
- Status: tombstone

Overrun is elapsed allowance, not retry permission. The barrier also blocks termination so an inactive run cannot strand its owned work. Wait or cancel, verify cleanup, then observe again. Inspect failed captures before retry; process success is not provider acceptance. New software-change implement graphs expose check-free revise-plan, revise-design and revise-intent without a report for rejected work. The driver selects the owner; no repository rollback or automatic invalidation occurs. Old stored graphs gain no edges.

For new runs, an unrelated holder of an old process or group number neither blocks progression nor receives cancellation signals. Genuine surviving owned work retains the barrier. Missing ownership evidence does not authorize signaling; historical runs are not migrated.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-111: Future execution corrections preserve frozen policy and past effective attempts.
- Status: tombstone

An observed current visit permits owner-attested amend-binding for one catalog slot using state_visit, owner, reason and a closed replacement binding. It changes future command/args/context_filter only, not initial input, topology, schemas, semantic policy or past attempts; an already-started attempt retains its commission. Show exposes original and effective bindings; invoke preview resolves the same preparation without primary launch. Ordinary max_active, force_fresh, timeout and applicable task/review selection require no amendment. Unsupported controls refuse. Fresh execution creates new captures without implicit standing reuse; selected tasks cannot claim unselected prerequisites ran fresh, and force-fresh review cannot use carried rows. This does not erase an arbitrary worker CLI's private sessions.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-112: Cancellation stops and verifies cleanup of only the current invocation-owned local process tree.
- Status: tombstone

Cancel-invocation validates run, current visit and recorded ownership, not arbitrary caller PIDs. Ownership is published behind a worker-start barrier; a durable stop marker under shared admission locking prevents later task/summarizer launch. The controller directly requests Dagu/direct-worker shutdown, escalates and verifies process disappearance/reaping before acknowledging terminal invocation failure. The run neither advances nor terminates and available captures survive. Each control acquisition/resumption has one monotonic ten-second deadline, at most three seconds graceful shutdown; phases do not reset it. Controller interruption remains incomplete: operator delay is unbounded, already-running work may persist, and later admission remains blocked. Retry resumes the same request with a new recorded ten-second attempt, never relabeling earlier interruption/timeout as success. Unverified cleanup keeps the barrier. Waiter loss does not defeat recorded ownership. Wrong-run/nonrunning targets without outstanding cancellation refuse without mutation; historical missing ownership is unsupported, not cancellable by assumption.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-113: Owner-attested exceptional progression remains permanently distinct from normal completion.
- Status: live
- Coverage: e2e/journey

Event --override names the current state_visit, owner, reason and one available edge after observation and quiescence. Stale/malformed/unavailable requests refuse without override history. Durable outcome overridden retains attestation, exact edge, known skipped bound checks and provider evaluation as not performed (not applicable for check-free). It does not invent unseen checks, artifacts, reviewer passes or Bookends GREEN, or rewrite denials and failures. Later edges require their own proof or separate exception. Show/list/history expose permanent has_overrides/count; final lifecycle stays final with completion_mode completed-with-overrides rather than completed.


**External-work successor qualification:** LE-167/168/171/178 retire executor-specific skipped-bound checks, owned-work barriers and launched task/summarizer packet requirements, not actual steering, exact-source dispositions, visible exceptional history or unresolved-findings refusal. Count eligible distinct actors under selected policy; explicitly permitted self-review does not override configured independence/cold eligibility. A count amendment neither discharges a failure nor converts an exception into normal completion.

### LE-114: Applicable durable software-change steering reaches only its named later commissions.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-115: Reasoned exact-source finding dispositions discharge failures without rewriting judgments or weakening author floors.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-116: Final software-change validation indexes complete independent current-criterion and whole-intent evidence at one checkpoint.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-117: Focused workers and bounded isolated proof jobs preserve complete final proof without duplicated suite ownership.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-118: Shipped software-change review construction defaults to one commission per used author per gate with distinct axis verdicts.
- Status: tombstone

Superseded by LE-130. The original title is retained for continuity and historical interpretation.

### LE-119: The public recovery surface adds amend-binding and cancel-invocation without adding provider semantic operations.
- Status: live
- Coverage: e2e/journey

The ten primary operations are start, list, show, append, event, history, terminate, invoke, amend-binding and cancel-invocation. Invocation-progress, fan-out and preview-bindings remain other commands with their existing catalog boundaries. Describe/evaluate remain the provider semantic interface; provider utilities perform no semantic judgment. New supporting software-change profiles declare semantic contract version 3 and the rigor/criterion policies in LE-130. Exact profile-version strings and encodings are maintained with the shipped data. The new provider explicitly refuses older semantic contracts; retain fixed old providers for old execution. Provider-free historical reads preserve original obligations and evidence meaning, with absent ownership/control metadata treated as absent capability. No active-run migration, compatibility scaffold or bootstrap rewrite is required.


**External-work successor qualification:** The supported primary engine operations are start, list, show, append, event, history and terminate. LE-167/176 retire primary work launch/control operations; describe/evaluate remain the semantic provider interface. Deterministic provider utilities prepare and inspect, never judge semantic truth or execute primary work. Successor contracts have explicit identities; old data stays unchanged, but no historical runtime availability/selection or executor support is promised. Unsupported historical semantic/detail operations may refuse explicitly; no active-run migration occurs.

### LE-120: A provider-agnostic Loop Engine monitoring command exposes ongoing run and external-work status and completion/attention notifications without model polling or workflow authority.
- Status: live
- Coverage: e2e/journey

Status distinguishes workflow, helper execution, worker outcome, output conformance and semantic judgment or its absence; missing, stale and conflicting evidence stays explicit. The same generic public interface supports different providers, run invocations, bound/unbound graphs and fan-out, and validation commands. A waiting caller can receive machine-consumable completion/attention notifications. Observer restart preserves execution/evidence and observation never approves, advances, retries or cancels work.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-121: Drivers can execute and capture serial proof matrices or one command with preserved failures, verified abort cleanup and explicit valid-prefix resume.
- Status: tombstone

The executor streams output and retains actual execution evidence, stops later admissions on failure, and never relabels earlier attempts. Resume refuses stale execution identity, missing evidence and unresolved cleanup. Execution success is not semantic approval; existing test commands remain the proof implementations.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-122: Optional iterative semantic status summaries remain evidence-grounded, cost-bounded and separate from deterministic monitoring and workflow authority.
- Status: tombstone

An operator-configured external completion command receives selected evidence/source locators and a fallible previous summary, explains changed evidence, explicitly corrects errors, retains prior outputs and discloses uncertainty and available usage/cost. No model API or harness is hard-coded. Missing, invalid or failed output never counts as a fresh summary; cadence and call limits bound automatic calls. Summary failure or budget exhaustion does not stop deterministic monitoring, work or alter workflow state.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-123: Software-change driver guidance exposes an explicit human/driver-owned Git checkpoint decision at the existing pre-review boundary.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-124: A separate post-commit delivery pointer can connect reviewed run/tree evidence to matching landed content and observed hosted outcomes without rewriting terminal evidence.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-125: Bookends continuity preserves full/shallow-clone parity and never treats unavailable required parent history as first adoption.
- Status: live
- Coverage: e2e/journey

Live-ID disappearance, tombstone removal, reassignment and revival are refused in both full and CI-equivalent shallow clones. The gate obtains required parent history or fails closed, while genuine first adoption remains valid; required push-to-main CI checks cover the introduced history required by LE-133.

### LE-126: Bookends bypass permission requires durable local invocation evidence and remains distinct from GREEN.
- Status: live
- Coverage: e2e/journey

Retain invocation time, repository/revision, bypass class/reason and outcome locally. Recording failure refuses bypass permission to push; normal GREEN/RED behavior is preserved and runtime evidence is not committed.

### LE-127: Bound model-worker context remains compact and usable without losing meaningful records or weakening durable evidence verification.
- Status: live
- Coverage: e2e/journey

Repeated engine-owned binding, preamble and schema evidence does not overwhelm meaningful bound model-worker context. Preserve original meaningful context IDs, judgments and ordering, current assignments and necessary instructions, full durable history and engine/provider verification evidence. Deterministic and other non-model consumers retain necessary data. Actual bound fan-out proof demonstrates compact delivered input and genuine evidence verification, including invalid-evidence refusal; separate plan-graph regression preserves correct inputs, output and evidence handling.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-128: Context-only appends preserve an in-flight evaluation's snapshot and outcome.
- Status: live
- Coverage: e2e/journey

Appending context while a checked evaluation runs does not by itself invalidate its allow or deny. That evaluation uses its original input snapshot. Later evaluations receive the appended records in durable order. A change to the source-state visit or lifecycle still invalidates an evaluation based on the old state. This replaces LE-38's exclusion from the guarantee.

### LE-129: Bound execution preserves actual worker outcomes, captured output and owned-work cleanup barriers.
- Status: tombstone

A completed invocation reports the actual bound command's exit outcome and retains available output and inner-worker results. Process success does not establish output conformance or semantic acceptance. Waiter loss without a durable terminal result is reported as failed or incomplete, and never proves that owned work has stopped. Surviving owned work and pending cleanup block retry and departure. Helper arrangement and direct parentage are implementation choices. The execution support remains invocation-scoped and requires no persistent daemon. This replaces LE-79; the historical topology mismatch remains part of the old audit.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-130: Software-change rigor levels enforce their declared review coverage and stages.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-131: Each software-change implementation task names the current acceptance criteria it serves.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-132: A small explicit worker roster produces inspectable per-run software-change setup.
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-133: Bookends checks every newly published reachable requirement transition.
- Status: live
- Coverage: e2e/journey

Required push-to-main CI inspects every commit introduced by the main ref update, starting from main's tip before the push (`before`) through the pushed tip (`after`), including merged branch commits, and checks continuity against each relevant parent. For a generic supplied update stream, Bookends inspects every supplied updated ref using that ref's own old/new range and checks every introduced reachable commit, including merged branch commits, against each relevant parent. No installed local Git hook or retired hosted workflow is required. Per-commit continuity (no silent removal, reassignment, revival, adoption removal, or tombstone games) still blocks wherever it appears, and incomplete history still fails closed. Coverage-timing findings on intermediate commits (eligible citations landing later in the same pushed range) are retained as visible diagnostics instead of blocking, provided the pushed tip tree is independently clean. Legitimate first adoption and permanent tombstones retain their meaning. Coverage of the published tip is checked against that tip's content. Known residual: landing an uncovered live ID then tombstoning it in the same range passes; tombstones are permanent and review-visible.

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
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-138: Generic fan-out preserves declared ordering, worker contracts and honest captures.
- Status: tombstone

Without an explicit barrier, workers remain parallel subject to the caller's concurrency bound. An optional barrier orders two worker groups while preserving stable assignment identities and selected execution. A conforming semantic failure may proceed to the independent later group. Execution and conformance failures remain visible at their respective boundaries. An uncontracted worker's nonzero exit remains recorded even when the facade exits zero; a declared conformance failure fails the facade. The engine interprets ordering and output shape only.

Existing compact bound framing, unchanged ad-hoc instruction bytes, declared legacy/full output contracts, true inner exits, retained attempts, mechanical join and failed-graph capture fallback remain supported. Captures are available before facade failure is reported. Process exit and conformance remain separate from semantic acceptance. No implicit workflow retry or semantic routing is introduced.

Fan-out retains its uncapped default when no concurrency bound is supplied. Plan-graph retains its default of four ordinary tasks, an explicit bound when supplied, and its summarizer after selected tasks. Exact argv, packet/framing bytes, scheduler encoding and capture fields belong to the CLI specification. This replaces LE-90's prohibition on inter-worker dependencies; it preserves the existing unbarriered behavior.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-139: Public-boundary journeys prove workflow and worker contracts without live models.
- Status: live
- Coverage: e2e/journey

Public-boundary journeys (`scripts/software-change-journey.py`, `scripts/policy-document-journey.py`, `scripts/research-journey.py`) freeze a sparse dummy-worker binding, invoke before the bound checked event, and prove catalog snapshot, instruction redaction, unbound-invoke rejection, pre-evaluate gate, worker-packet stdin, overlay `succeeded`, unbound stored instructions, and invocation history. Software-change journeys also prove unbound shipped profiles, graph-runner and fan-out behavior with dummy inner workers, `preview-bindings` nonzero on zero-worker `fan-out` JSON without creating a run, `preview-bindings` warning when pi has `--no-extensions` and no `-e`, and do not call a live model. `scripts/software-change-journey.py --self-test` executes the software-change setup utility and two provider skill constructors against software-change high-rigor policy/stage setup, policy-document shipped semantic policies/target/mode, and research verify plus synthesize; it compares worker count/order and exact axis/`example_prompt`/author/model/subject metadata to each source profile, asserts required keys/data bytes/preview visibility and fail-closed invalid cases, asserts root AGENTS rules, and prints `worker-data skill/root policy assertions passed` only after all pass. The software-change source full journey binds deterministic stdin-capturing workers that emit conforming JSON or exit-0 refusal text and, through separate public CLI processes, asserts the compact one-key `artifact_root` context precedes the separator/body with no `capture_dir` or duplicate identity in that block, conforming `status` succeeded, refusal `status` failed with `exit_code` 0, summary/captures persist, overlay fails, then prints `contracted fan-out failure`.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-140: Opt-in review bindings use the extensions their selected model providers require.
- Status: tombstone

Bound review slots frozen to `fan-out` still require `loop-engine invoke RUN_ID SLOT_ID`. A usable review binding contains provider-constructed assigned `--worker` objects frozen at `start` after `preview-bindings` and lock-in; a review slot with an empty configured policy-axis list is not bound. Shipped profiles omit `work_slot_bindings` so slots stay driver-performed. Opt-in skill templates keep `--no-extensions` and add an existing `-e` path for each extension required by the selected model provider; omitted paths produce no `-e` pair. Default implement inner argv when `--task-worker` is omitted remains `pi --print --no-skills --no-extensions` and must not pass `--no-context-files`. A supplied extension path must be absolute and exist.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-141: Proposed acceptance-granularity software-change intent-review obligation
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-142: Proposed software-change reconciliation state and semantic coverage
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-143: Proposed owner-facing proactive software-change updates
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-144: Proposed owner-comprehensible software-change intent obligation
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-145: Assignment-level honest progress
- Status: live
- Coverage: e2e/journey

A provider-neutral local view identifies each review assignment by stable axis and author, and other assignments by stable task ID/title/role, from frozen caller-supplied labels rather than inferred worker text. Human and machine drill-down share the same identity and evidenced queued, running, correcting, conforming, failed/exhausted, cancelled or unknown state. Current-subject-revision attempts, repeated failures and actual errors exclude older failures, which remain inspectable. A quiet worker is not diagnosed dead; process completion/conformance is not semantic approval, and observation never retries or cancels work.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-146: Bounded honest local status
- Status: live
- Coverage: e2e/journey

An ordinary on-demand local status read, including a mature mixed run, reaches last output byte within two seconds. A slow, locked or incomplete source returns useful available facts and marks each affected lane partial, stale or unavailable rather than scanning unbounded history or inventing certainty. Complete historical and original output remain retrievable separately.

### LE-147: Provider-neutral navigable workflow status
- Status: live
- Coverage: e2e/journey

A read-only local terminal view presents the frozen provider-defined graph, current state first, all states and assignments via basic keys and scrolling, and keeps selection through portrait folded/unfolded phone resize. Visited history is distinct from actual requestable routes, and checked requestability is not approval. Depict only real invocation barriers/dependencies; when no graph exists, list the work instead. Rendering stays in the CLI, not core workflow policy. Representative real-device usability requires owner judgment.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-148: Focused actionable reads with retained detail
- Status: live
- Coverage: e2e/journey

Current-action inspection provides duties, constraints, actual available actions and targeted source/error/capture locators without a whole-run dump. Read-only delta and assignment/error selection expose actual failures and original paged stdout/stderr with explicit offsets, totals and truncation; recent/targeted and full retained history remain available. Measure lower delivered bytes and latency on active mixed-state agent read paths without hiding the fact needed for a correct next action. Version changed views and update supported consumers; observation is not a prove-you-read-it gate.

**Context headers and exact records:** `read --kind context` returns paged headers for a run's context records (id, kind, sequence, stored byte size and a short preview of top-level scalar fields) and `read --kind record --id ID [--pointer P]` returns one stored record exactly, or a JSON-Pointer slice of it, so finding or citing a record needs no whole-run payload. `history` entries for added context name the record's kind, and `list` accepts lifecycle, label and provider filters. These reads do not arm mutation, store anything, or assign meaning to a record (LE-18); unfiltered `list` and the existing `history` fields are unchanged.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-149: Review-necessary context and driver-owned fit
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-150: Applicable work survives same-run plan reshaping
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-151: Attributed output-only recovery without new judgments
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-152: Explicit partial-invocation recovery
- Status: tombstone

After failed or cancelled fan-out, expose stable IDs for verifiably complete siblings, invalid/incomplete members and never-started downstream members. Following verified owned-work quiescence, the driver explicitly selects necessary reruns or a genuine same-binding mechanical recovery join. Joined selected sources retain their failed/cancelled raw origins, source class and original captures while the old invocation stays failed/cancelled; a new succeeded outer invocation is never represented as new reviewer judgment. Real barriers, selected bytes and current applicability still govern checked admission and required fresh review. A substantive reviewer fail is not an execution retry; no automatic whole-gate restart or individual-worker cancellation is required.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-153: Honest narrow driver-owned correction and same-graph final report
- Status: tombstone

Only a graph explicitly opting a **non-review draft or implement slot** in may accept a small understood driver-authored code or workflow-artifact correction when accepted intent, design and decomposition are unchanged and no conflicting work owns it. Record current visit, binding, author, changed work and reason as a distinct non-worker pre-evaluation completion act after quiescence; still run the normal checked provider/artifact/evidence/Bookends gate and refresh affected proof and independent reviews. Never fabricate bound success, edit raw attempts, perform reviewer work by driver act or use the general override as ordinary repair. Larger or uncertain changes return to their owner/phase. After an actual LE-142 decision and authorized document changes, a separate future report-only selection on the **same bound plan graph**, if all tasks still stand, runs only its frozen summarizer and generates a fresh current-tree report/checkpoint before re-entry; affected real tasks instead use supported task-root selection. Neither selector migrates an existing frozen run, authorizes Git, or labels a pre-decision report final. Where no supported report path and no genuine affected task work exist, stop and escalate rather than manufacture task work.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-154: Invalid reuse cannot erase valid fresh siblings
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-155: Grounded passes and challengeable reasons
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-156: Source-faithful intent and truthful approval
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-157: Merit-based finding resolution and visible exception
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-158: Preview and exact apply of factual evidence
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-159: Per-criterion distinguishing plan-time proof
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-160: Standalone editable profile examples
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-161: Phase-appropriate five-artifact Bookends coverage
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-162: Early Bookends prerequisite visibility
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-163: Generic optional typed advisory invocation
- Status: live
- Coverage: e2e/journey

A configured provider-reusable command receives one structured JSON state and independently keyed questions, with exact-key typed answers and separately retained original input, command, response and failure; no backend or model is selected by the engine, and advice cannot complete primary work or grant workflow authority. Choice supplies selected named option key, finite normalized full probabilities over exactly the supplied options and confidence in [0,1]. Score supplies a probability-weighted value over ordered rubric-level indices, a matching indexed level-to-description legend, finite normalized full level probabilities and confidence in [0,1]. Noul supplies a yes-probability in [0,1], **without mandatory separate confidence**. Each type permits absent or present per-answer string rationale. No fourth abstain type, invented missing probability/field, required prose, vendor HTTP envelope or named backend follows. A generic non-software provider can use the same interface.

Every advisory question must be one bounded judgment over sufficient explicitly supplied evidence. Independent questions may be batched; questions requiring investigation, missing-fact inference, planning, multi-step reasoning, or whole-workflow synthesis must not be sent to an advisor. Those tasks remain with the coordinating driver; there is no automatic reasoning-model fallback through the advice feature.

The driver is responsible for deciding whether a question and its evidence meet that boundary before invocation. Mechanical checks may reject missing or malformed inputs, but must not claim to prove semantic sufficiency. Shipped question builders must not generate the excluded classes of question.

Advice remains fallible and non-authoritative regardless of confidence. The driver accepts, partially accepts, or rejects each actual answer with a reason.


**External-work successor qualification:** LE-167/172/176 remove advisor backend command/configuration, invocation and attempt dispatch, not configured substantive typed questions, choice/full probabilities/confidence, score/legend/full probabilities/confidence and Noul probability with optional confidence/rationale. Genuine externally produced answers and actual accept/partial/reject dispositions close configured occasions; a permitted no-admissible-question reason stays explicit. Missing required values cannot be projected into existence. Normal checked/check-free departure closure and visible scoped exception/refusal remain; advice is neither review approval nor proof/route authority.

### LE-164: Software-change advice for eight evidenced decisions
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-165: Explicit advisory enablement and accountable occasions
- Status: live
- Coverage: e2e/journey

All three standalone software-change examples name the same eight occasions, without a default command/backend. Setup makes the choice to configure a conforming command or decline explicit and shows the effective enabled/disabled state before start. On enabled active runs, guidance names due request and disposition at actual review candidates, accepted defects, needed implementation corrections, execution/authority issues, declared evidence reuse, reconciliation, review-round departure and final completion; an ACTIVE driver can also request ad hoc advice. Every successfully returned answer, including superseded and ad hoc answers, receives an individually reasoned accept/partial/reject disposition. Rejecting wrong advice is ordinary, not an exception; there is no extra general draft-readiness checkpoint.


**External-work successor qualification:** LE-167/172/176 remove advisor backend command/configuration, invocation and attempt dispatch, not configured substantive typed questions, choice/full probabilities/confidence, score/legend/full probabilities/confidence and Noul probability with optional confidence/rationale. Genuine externally produced answers and actual accept/partial/reject dispositions close configured occasions; a permitted no-admissible-question reason stays explicit. Missing required values cannot be projected into existence. Normal checked/check-free departure closure and visible scoped exception/refusal remain; advice is neither review approval nor proof/route authority.

### LE-166: Advisory prerequisite at normal workflow departure
- Status: live
- Coverage: e2e/journey

When a configured occasion has no admissible advisory question, the driver records why no call is appropriate and continues through the unchanged ordinary checks and authorization rules. This is not a fabricated advisor response or permission to bypass those checks. A failed eligible advice call retains its existing failure and recovery rules.

Only on new runs explicitly enabling a frozen advisory departure map, every relevant normal **checked or check-free workflow transition** requires valid admissible due advice and reasoned dispositions of all successful answers (including ad hoc and superseded). A changed target requires explicit driver applicability or fresh advice. A failed, timed-out or invalid required response is no answer and blocks that prerequisite until valid response or a scoped owner-attested exception for precisely unanswered due occasions. Even an exception cannot waive a successful answer's missing disposition, a provider/reviewer/criterion/Bookends/proof gate or owned-work cleanup; retain exceptional history visibly. A reasoned reject of wrong advice permits ordinary progression. Earlier edit, append and invoke timing is guidance, not an action interlock; cancellation/cleanup do not wait for advice. Disabled graphs and older frozen runs retain their original prerequisites; no existing run is migrated.


**External-work successor qualification:** LE-167/172/176 remove advisor backend command/configuration, invocation and attempt dispatch, not configured substantive typed questions, choice/full probabilities/confidence, score/legend/full probabilities/confidence and Noul probability with optional confidence/rationale. Genuine externally produced answers and actual accept/partial/reject dispositions close configured occasions; a permitted no-admissible-question reason stays explicit. Missing required values cannot be projected into existence. Normal checked/check-free departure closure and visible scoped exception/refusal remain; advice is neither review approval nor proof/route authority.

**Running-advice qualification:** For new runs on a release supporting LE-192, owner-attributed advice-policy-change records may replace the effective departure map during the run, including an advice-off start. The latest complete map governs the currently pending departure; addition makes a requirement due and removal releases unanswered advice. The frozen original and every change remain visible. LE-189 permits explicit same-visit occasion revision without an owner ceremony. Neither policy change nor occasion revision removes a successful received answer's reasoned-disposition duty. Earlier frozen runtimes retain their original prerequisites and are not migrated.

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

## 19. External-work amendment

### LE-167: External execution across all providers
- Status: live
- Coverage: e2e/journey

Software-change, policy-document and research can complete their configured workflows with the driver/harness performing or commissioning work, including running proof/test commands. Engine/provider agent launching and primary command-work execution or supervision are not required or supported.

Agent execution bindings, invocation lifecycle, fan-out execution and plan-graph execution are removed from the new path—not recreated as another scheduler or mandatory start/finish protocol. Deterministic provider describe/evaluate calls and necessary deterministic metadata checks remain. The engine records supplied inputs, results, references and metadata; it does not launch proof commands or matrices under a capture-command, capture-matrix or equivalent work-runner surface. Recording supplied output is not proof that the engine executed it. Normal external command-proof fulfillment is specified in LE-183.

### LE-168: Honest, configurable review coverage
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-169: Useful plans without mandatory agent isolation
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-170: Reliable guided recording and correction
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-171: Ordinary, proportionate reuse
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-172: External advice with preserved configured obligations
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-173: Intent sufficiently constrains the intended final state
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-174: Review briefs can be prepared early without advancing the workflow
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-175: Acceptance criteria require the promised result
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-176: Retired agent and proof-command execution machinery no longer imposes a supported maintenance responsibility
- Status: live
- Coverage: e2e/journey

The refactor retires agent execution from the supported engine/provider product, not only its ordinary new-run path. Obsolete agent-execution commands and dedicated implementation, configuration/binding controls, dependencies, and execution-only tests and documentation are removed rather than kept behind a bypass, optional executor or actively supported historical path. This covers invocation launch/waiter lifecycle, agent supervision/cancellation/progress controls, fan-out/plan-runner scheduling and joining, agent-output execution recovery, agent-advice dispatch, and proof-command/matrix launch, execution supervision and cleanup/control surfaces wherever their remaining purpose is that retired responsibility.

Shared facilities with an actual remaining supported use in durable workflow/provider evaluation, evidence recording, faithful external-return handling, or deterministic provider/metadata evaluation and recording supplied command-proof results remain. Shared process, timeout, cancellation, schema, persistence and input/output-recording code is not deleted merely because a retired executor also used it; it must have an actual remaining non-work-execution use. Dependency removal is limited to dependencies with no remaining supported use. Minimal inert historical representations may remain where needed to preserve existing records without continuing old agent execution; historical records and captures are not deleted, reset, migrated or silently reinterpreted.

Acceptance distinguishes retirement from a merely functional bypass: public command/help/configuration surfaces, dependency/runtime prerequisites and source/test/documentation inspection agree that the obsolete agent and primary proof-command execution machinery is no longer supported or maintained. Old-history interpretation remains truthful or explicitly unsupported, and retained provider/workflow/evidence/externally-produced-proof paths still work. No automatic dead-code classifier, exhaustive provenance inventory or unrelated repository cleanup is required.

### LE-177: Qualifying early evidence remains usable when progression reaches its review state
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-178: Explicit, durable required-author-count amendment
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-179: Honest final residual reporting through existing surfaces
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-180: Small profile customization without owning a wholesale fork
- Status: live
- Coverage: e2e/journey

The guaranteed small-customization categories are common supported policy knobs: required author counts for reviews, criteria and the whole goal; explicit eligible self-review permissions; review groups/axis membership and individual/aggregate stage selection; and advice and Bookends switches. These can be expressed through the base-derived preparation path without owning the complete base file. Raw artifact-schema or prompt replacement may still use a complete validated custom profile; this criterion does not require a general editor for every possible field. These are pre-start policy choices, not permission to change semantic group/stage/eligibility policy in an active run.

Before starting a supported provider run, an operator can select a shipped base profile and express a small set of intended changes through supported preparation, without hand-maintaining the entire copied default profile. The effective profile is validated and inspectable with its actual stages, groups, author counts, eligibility, advice and other relevant supported obligations. The operator can distinguish their overrides from the base defaults and see the base identity and any drift or divergence; a custom name cannot silently select different semantics.

For future preparation using a newer shipped base, the unoverridden remainder comes from that selected base rather than a stale wholesale copy. Changed defaults, conflicting or no-longer-applicable overrides remain visible rather than being silently accepted. Resolve and confirm the actual effective profile before launch. A started run does not silently track subsequent shipped changes: its original resolved input remains recorded, with only separately supported explicit amendments such as LE-178 changing active counts.

Preserve required independence and self-review rules except choices expressly made through supported pre-start policy. Use the smallest adequate recipe/options/overlay path, not an inheritance framework, preference store, editor, automatic active-run migration or execution binding system.

### LE-181: One workflow can deliver across multiple repositories
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-182: Clear ordinary findings before challenging the clean conclusion
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-183: Externally executed command results fulfill normal proof obligations
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-184: Software-change plans expose safe real parallelism and plan review checks it
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-185: Optional reviewed narratives and owner checkpoints for software-change runs
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-186: Approved run artifacts remain inspectable after run files are deleted
- Status: live
- Coverage: e2e/journey

For new runs of the shipped software-change, research and policy-document providers, each time a run leaves an artifact's owning phase forward, the exact content of that artifact as it stood at that moment is retained in the run catalog. Deleting the run's artifact directory, or any other file outside the catalog, does not lose that content: an operator can read every retained copy, in order, with its artifact name, the transition that retained it and when, through the ordinary inspection commands.

Only phase exits are retained, not intermediate drafts or revisions written between exits. A later exit after a revise loop adds a new entry; an artifact whose bytes are unchanged since its last retained copy is recorded as exited again without storing its content a second time. An exit by owner override is retained and visibly marked as not checked. Core assigns this content no artifact, approval or revision meaning; runs started before this capability are not backfilled.

### LE-187: Operators can reclaim legacy catalog space from finished runs
- Status: live
- Coverage: e2e/journey

An operator can deliberately reclaim catalog space held by bulky per-invocation detail that earlier releases stored for work-slot invocations, such as worker command, argument and prompt payloads and their duplicated completion snapshots. Before changing anything the operator can see which finished runs would be affected and roughly how much space would be reclaimed. Applying it discards that detail only for final or terminated runs, keeps those runs listed, and returns the freed space so the catalog file on disk becomes smaller.

Active runs and their data are unaffected, and cleanup does not refuse merely because other runs are active. While the catalog file is being shrunk, concurrent commands may briefly wait or fail with a clear retryable busy error, but nothing is corrupted or lost beyond the selected legacy detail. Inspection of a cleaned run may be degraded; original worker payloads for cleaned runs, and verification that depends on them, are intentionally no longer available. Cleanup never runs automatically.

### LE-188: Context writes refuse records provably invalid at write time
- Status: live
- Coverage: e2e/journey

For runs started on a release that provides it, every public context write, including direct append, refuses before persistence a record that can be deterministically proved invalid from the record, earlier records, frozen run policy and sources present at write time. The refusal names the record and the failing rule, and nothing is stored. Engine storage integrity (active observed run, unique record identity, append order and existence of a referenced origin) is checked first and is never waived. A record that carries or wraps another record, such as evidence applicability, is also checked against the rules that apply to what it carries. When required validation is unavailable or incompatible, the write fails closed. Preview helpers, writes and later evaluation apply one effective rule set, including obligations a frozen overlay adds, so a record previewed clean is not later refused for a reason checkable at write time. The engine stays provider-generic and assigns context no meaning (LE-18): provider-owned record kinds follow their provider's rules, engine-owned kinds follow engine rules, and semantic judgment is not validation. There is no exception that stores a refused record; a rule defect is corrected in the rule. A record valid when written that later becomes obsolete follows existing revision and applicability rules. Runs started earlier keep their frozen obligations.

### LE-189: Advice-occasion records are checked at write and revisable within a visit
- Status: live
- Coverage: e2e/journey

On advice-enabled runs the engine refuses an accidental duplicate or provably invalid advice-occasion record when written, naming any existing record. The driver may explicitly revise its own advice-occasion record within a state visit; the latest explicit revision governs that departure, originals remain in history, and successful answers keep their reasoned-disposition obligation.

### LE-190: Obsolete records can be withdrawn without rewriting history
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-191: Commissions report bad records instead of aborting
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-192: Owner can change advice requirements on a running run
- Status: live
- Coverage: e2e/journey

The owner may enable or disable advisory mode and add or remove individual advice requirements on an active run without restarting it. A change applies from the moment it is recorded, including the pending departure: removal releases unanswered due advice and addition makes advice due before departure. Received successful answers keep their reasoned-disposition obligation. The frozen original and every change remain visible, and show exposes the effective advice policy. This qualifies LE-166's start-frozen advice map for runs on releases that support it.

### LE-193: Software-change runs can return directly to any earlier phase draft
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-194: Ordinary-review writes are refused while that review's challenge is under way
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-195: Model bindings in the per-run profile
- Status: live
- Coverage: e2e/journey

A software-change per-run profile MAY carry an optional `model_bindings` object holding, for any subset of the roles driver, author, implementer, ordinary-reviewer, challenge-reviewer, cold-reader and criterion-author, one required `provider/model` string and one thinking-level string, plus the driver's own declared session when the driver is bound. Both values MUST be nonblank. The bindings are part of the exact profile bytes and identity shown to the owner before start, are frozen with the run input, and are not changed by later profile-file edits. Start MUST refuse a malformed `model_bindings` value or a missing or unequal driver declaration, naming the cause. Shipped profiles MUST carry no bindings and choose no models.

### LE-196: Declared sessions on stored work
- Status: live
- Coverage: e2e/journey

For every role with a binding in force, the session producing the work MUST declare the provider/model and thinking level it ran with, as read from its own session metadata: on each review judgment, challenge judgment, cold-reader reconstruction and criterion or goal verdict; on a `session-declaration` for each author draft and implementer report of the exact subject revision; and from the driver at run start and in each binding amendment, but not on every command or append. A role with no binding in force needs no declaration. Declared values are compared as exact text with no aliasing or normalization. The engine and provider judge declared values only; they neither authenticate the declarer nor verify the real model.

### LE-197: Refusal of missing or mismatched declarations
- Status: live
- Coverage: e2e/journey

A declaration that is missing, or whose provider/model or thinking level differs from the binding in force for its role, MUST be refused at the point the run would first store or accept the work (append for returns, the checked step for drafts and reports, run start or amendment for the driver). The diagnostic MUST name the role, the expected and actual values (or that the statement is missing) and which part differs. Refused work is never stored or counted toward a gate, author floor or checkpoint, and context and state are unchanged. Where the refusal is a checked-event denial (author drafts, implementer reports), the engine's ordinary denied-transition record is the only history entry and no accepted record or transition is added; a refused append adds no history. Records stored earlier are never re-judged; each remains judged against the binding in force when it was stored.

### LE-198: Owner binding amendment
- Status: live
- Coverage: e2e/journey

The owner MAY add, change or remove bindings of one or more roles on a running software- change run, including a run started with none, by appending a `model-binding-amendment` record carrying a human actor, owner authority, the owner's verbatim nonblank words, each affected role with its old binding (none when adding) and new binding (none when removing), the driver's declaration whenever a driver binding is in force afterward, and an optional stale mark. The amendment takes effect at its own position in history, cannot be back-dated, and rewrites nothing. It MUST be refused, with the run unchanged and the cause named, when: the actor is not human; the authority is not owner; the words are blank; a role is unknown; a stated old binding is not the binding in force (including adding to a bound role or changing or removing an unbound one); a new binding is malformed; the driver declaration is missing or unequal to the driver binding in force afterward; or evidence is marked stale for a role other than ordinary-reviewer, challenge-reviewer, cold-reader or criterion-author. Standing delegation does not authorize an amendment.

### LE-199: Eligibility of evidence across amendments
- Status: live
- Coverage: e2e/journey

By default evidence produced under a superseded, changed or removed binding, or while the role was unbound, remains eligible exactly as it would have without the amendment, including carrying by explicit evidence-applicability and group coverage by returns made under different bindings. An amendment that marks a role stale makes that role's earlier returns ineligible for decisions requested after it and uncarryable to new targets, without reopening earlier decisions or changing any original record. Commission and the checked event MUST give the same eligible-or-refused answer, and a refusal MUST name the amendment, role, author, assessment and axis. Author floors, author independence, assessment identity and the coverage of every axis are unchanged.

### LE-200: Visibility and unbound behavior
- Status: live
- Coverage: e2e/journey

The `show` status and full views and the owner approval view prepared by commission MUST show, per role, the original binding (or none), the binding in force (or none), and the ordered amendment history (time, actor, verbatim words, old and new binding, stale mark), and MUST say explicitly when a run never had bindings and has no amendment. A run with no bindings and no amendment, including one created from unmodified earlier profile data, MUST behave exactly as before: no model statement, no new refusal. Runs started by earlier builds are not changed.

Accepted limits for LE-195 to LE-200:

- Declared values are the boundary: a wrong declaration equal to the binding passes.
- The driver declares only at run start and in each amendment; a model change between
  those points is not detected.
- Superseded-binding evidence stays usable unless the owner's amendment marks it stale.
- Interaction with the unmerged evidence-carry fix (#30, #31, #40, #41) is rechecked at
  integration.

### LE-201: An owner can relax, defer, withdraw or clarify a reviewed requirement without a revision
- Status: tombstone

Retired with the software-change provider, which lean-change replaced (LE-202). The original title is retained for continuity and historical interpretation.

### LE-202: The lean-change provider has one workflow in two profiles that differ only by the review state
- Status: live
- Coverage: e2e/journey

The `lean-change` provider (binary `lean-change`, not released) describes a workflow of explore, intent-approval, plan, implement, verify, review and end. Profile `b` freezes `review: true` and keeps all seven states (twelve transitions). Profile `c` freezes `review: false` and describes the same workflow without the review state: verify routes to end (six states, ten transitions). Every other state and edge is identical in both profiles, and so is every gate except `intent-ready`, which profile `b` also gates on the intent check (LE-209). A run frozen under one profile is never evaluated as the other. The provider is a checker; the driver does the work and appends the records named below with `append`. It reuses the engine and the generic owner-decision and repository-identity modules and changes neither.

### LE-203: Setup freezes the repository, its base and the principles; the intent names an executable check for each criterion
- Status: live
- Coverage: e2e/journey

`lean-change setup --profile b|c --repo PATH [--principles FILE]` prints the initial input: schema version, profile version, absolute repository path, the base commit, the review flag and, when given, the principles text with its sha256. Profile `b` refuses to set up without principles; the input refuses a principles text that does not match its hash. `explore` leaves only on a valid latest `intent` record: an author, a goal, constraints, a `hard` flag and at least one criterion, each with a unique id, a statement and an executable check (argv and a timeout). A missing or invalid intent is refused.

### LE-204: Only the owner's verbatim words, bound to the current intent record, approve an intent
- Status: live
- Coverage: e2e/journey

`intent-approval` leaves by `approved` (hard=false, to implement) or `approved-hard` (hard=true, to plan) only when the latest well-formed `owner-decision` (checkpoint `intent`, subject `intent`) approves the latest intent record id and holds nonempty verbatim words. Blank words, a decision about another intent record, and an intent revised after the decision leave the approval pending and the event refused. A recorded rejection is refused and names its reason. The event that does not match the `hard` flag is refused. `revise` returns to explore without a check.

### LE-205: Hard work needs a plan for the current intent
- Status: live
- Coverage: e2e/journey

`plan` leaves by `planned` only when the latest `plan` record names the current intent record and has nonempty text. Work that is not hard skips the plan state. The plan is not reviewed.

### LE-206: Verify needs every criterion's approved check to pass on the current tree
- Status: live
- Coverage: e2e/journey

`lean-change verify` reads the full `show` envelope, runs each criterion's check in the repository with its timeout, and prints a `verification` record: the intent record id, the repository proof identity after the checks, and per criterion the check's sha256, exit code, timeout flag and output tail. `verified` is refused unless the current intent record is approved by the owner (LE-204) and, when hard, has its plan (LE-205); a later intent record therefore never inherits an earlier approval, plan or verification. It is also refused unless the latest verification names the current intent record, its tree equals the current repository proof identity, and every criterion has a result for its approved check with exit code 0 and no timeout. Each check runs in its own process group with its output reduced to a bounded tail; the group is killed at the timeout and after exit. If the checks leave different trees (a later check edits what an earlier one relied on) after two passes, `verify` prints no record. The repository proof identity covers tracked and not-ignored files; git-ignored files are outside it. A missing verification, a missing criterion, a failed or timed-out check, a check other than the approved one and any edit after the verification are each refused with the criterion and reason.

### LE-207: Profile b needs one independent, fresh-context diff review with six fixed questions and every finding disposed
- Status: live
- Coverage: e2e/journey

`lean-change review-packet` prints the six fixed questions (intent, scope, edge-cases, checks, docs, principles), the approved intent, the frozen principles text and sha256, the checks that passed with the tree they vouch for (a packet for a stale tree is refused), and the diff of the tree against the base. The principles question applies the owner's design principles; the Gate lines apply to new components and new cross-component boundaries, not to small changes inside an existing component, and a finding without an obligation, evidence and consequence is not a finding. `reviewed` is refused unless the verification is current (LE-206) and the first well-formed `review` record for the current intent (a later review cannot replace it) has a reviewer other than the intent author, `fresh_context: true`, one noted answer to each question and no other, a finding for every failed question, each finding with an id, question, text, evidence and consequence, and a `finding-disposition` (fixed or rejected, with a reason; the latest wins) for every finding. If the tree changed after the review, at least one finding must be disposed as fixed. There is one review: later edits are answered by re-verification, not a second review. Profile `c` has no review state.

### LE-208: Accepted limits of the lean-change provider
- Status: live
- Coverage: e2e/journey

The provider records claims, not authentication: reviewer identity, fresh context and the owner's words are as honest as the driver that appends them. Criteria have executable checks only; there are no manual criteria. A check can be too weak: the review's `checks` question is the only defence, and profile `c` has none. Review is one pass; edits after it are checked only by re-verification. The topology cannot depend on context, so hard work uses two approval events. A check that starts a process outside its process group is not contained. The Loop Engine start confirmation by the owner is driver procedure, not a provider gate. Software-change keeps its own requirements; this provider adds none to it.

### LE-209: Profile b needs one independent, fresh-context intent check with four fixed questions and every finding disposed
- Status: live
- Coverage: e2e/journey

`lean-change intent-packet` prints the draft intent, the repository path and the four fixed intent-check questions: "Once this exists, what will the owner or its users need to see or do with it, and which of those can they not do yet under this intent?" (use); "What is the likeliest case the listed checks miss, and what should happen then?" (edge); "Where is this intent unclear or silent, such that a delivery would have to guess?" (control); "Which term in this intent could mean two things to different readers, and which meaning do you intend?" (terms). `intent-ready` is refused in profile b unless the first well-formed `intent-check` record (a later one cannot replace it) names an intent record, has a reviewer other than the intent author, `fresh_context: true`, one noted answer to each question and no other, a finding for every failed question (its text is the concrete question for the owner), each finding with an id, question, text, evidence and consequence, and a `finding-disposition` (fixed or rejected, with a reason; the latest wins; `review` names the intent-check record) for every finding. If any finding was fixed, the current intent record must be a newer one than the intent checked. The check runs once per run; a later revision is not re-checked. The skill has the driver ask the owner "What do you expect that is not listed?" when it shows them the intent and the findings. Profile `c` has no intent check.
