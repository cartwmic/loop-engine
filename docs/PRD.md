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

These records govern applicable behavior of software-change, policy-document and research. Provider-specific target/phase meaning remains. Policy-document gains explicit configured groups/counts/eligibility and same-run applicable evidence rather than relying on its former fixed one-pass/no-floor rule; its actual target authorship is a declaration, not a Git-committer inference or registry. Research's adversarial question remains contrary-evidence checking, not an invented software-change phase. Actual requests/returns can be prose/YAML/JSON; strict normalized projection belongs to supported driver recording, not a mandatory reviewer output schema.

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
- Status: live
- Coverage: e2e/journey

### LE-92: Reviewer output is candidate evidence only. The driver owns the append-only current `finding-ledger` disposition and routing view: an advisory classification proposal is inert until accepted or edited, and only current accepted unresolved implementation findings explicitly routed to a task or review axis affect packets or later gates; raw candidate sources and driver decisions remain distinguishable.
- Status: live
- Coverage: e2e/journey

### LE-93: A bound reviewer using `full_output_schema` receives at most one correction opportunity under the identical frozen command, arguments, assignment, preamble, and model. The first and second raw stdout/stderr attempts, exact validation errors, selected attempt, and exhaustion state remain retrievable; a second invalid response fails closed without a substitute reviewer or semantic verdict.
- Status: live
- Coverage: e2e/journey

For new LE-151 output-only recovery, an optional identical-worker retry is a **new substantive judgment**, not a prerequisite to safe mechanical or configured bounded representation-only repair of explicit original meaning. Neither kind of correction rewrites original attempts.


**External-work successor qualification:** LE-170/177 replace new evidence dependence on same-worker retry, engine invocation/assignment selection and captured packets with actual supplied requests/instructions, original returns and faithful separately attributed driver-confirmed projection. Preserve known identity/content checks, original result/author/findings/rationale, visible gaps and inert candidate previews. Valid siblings survive malformed siblings; missing substantive meaning is not formatting. No engine-executed claim or new work registry is created.

### LE-94: `implementation-ready` cannot advance from an author-declared implementation report alone. The provider-generated implementation checkpoint independently binds the report and document revisions to the current repository HEAD, index, status, tracked/non-ignored-untracked entries, and content identity, and the public checkpoint command is read-only with respect to Git.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-171/181/183 retain genuine report/checkpoint and accepted proof identity while admitting scope-sensitive applicable retained work and genuine external command proof. Source/command identity and current affected checks remain; multiple declared repositories and their authorities are checked separately. Wrong/stale/missing/failed proof refuses. Historical source identities are never rewritten to current hashes, and primary success cannot satisfy secondary work.

### LE-95: After implementation or final validation proof is accepted, changing repository HEAD, adding or deleting a tracked or non-ignored untracked entry, renaming an entry, changing status or type, or changing tracked bytes makes the affected checkpoint stale and refuses later checked progression until proof is regenerated. The checked transition that admits implementation to validation records the exact checkpoint under content-addressed `implementation-proof-history/`, whether or not implementation review is configured. Validation requires the sole accepted entry for the current report revision to match the current report, document revisions, and repository state. Appending later context, overwriting history bytes, or replacing both mutable checkpoint files cannot admit bytes that transition did not accept.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-171/181/183 retain genuine report/checkpoint and accepted proof identity while admitting scope-sensitive applicable retained work and genuine external command proof. Source/command identity and current affected checks remain; multiple declared repositories and their authorities are checked separately. Wrong/stale/missing/failed proof refuses. Historical source identities are never rewritten to current hashes, and primary success cannot satisfy secondary work.

### LE-96: When validation exposes a stale repository checkpoint, the validation draft and review states expose a check-free `revise-implementation` recovery route. Final approval requires regenerated implementation and validation checkpoints for the same current tree plus passing review evidence; validation cannot silently replace implementation proof.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-171/181/183 retain genuine report/checkpoint and accepted proof identity while admitting scope-sensitive applicable retained work and genuine external command proof. Source/command identity and current affected checks remain; multiple declared repositories and their authorities are checked separately. Wrong/stale/missing/failed proof refuses. Historical source identities are never rewritten to current hashes, and primary success cannot satisfy secondary work.

### LE-97: Plan and validation review require observable user or operator outcomes, pragmatic black-box proof or a concrete impracticality reason, and semantic inspection of every new or changed Bookends citation. A final validation report maps requirements to observable proof, and passing activity or a matching requirement token alone is not treated as completion.
- Status: live
- Coverage: e2e/journey

### LE-98: `show` of current state and instructions is the observation that arms mutation. `append`, `event`, `invoke`, and `terminate` refuse without a current observation, preserve state and semantic history on refusal, and require a new observation after every state visit, including a self-loop; `list`, `history`, and `invocation-progress` do not arm mutation.
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-167/176 retire binding, invocation, waiter, owned-work and mandatory execution-detail projections, not durable provider-free state/input/context/history, observation, one logical mutator or external completion. Historical detail may be explicitly unsupported without deleting or reinterpreting stored data. Frozen normalized counts are identified as base values; supported provider inspection resolves effective counts under LE-178 and is exposed by guidance for fresh continuation under LE-20/21.

### LE-99: A completed bound invocation durably identifies each enumerable assignment independently, including selected attempt or coverage gap, the digest of selected originating bytes, and their originating-attempt location. A worker-level copy is not the selected-attempt identity, and these facts remain inert until a driver acts on them.
- Status: live
- Coverage: e2e/journey

In new LE-151/152 recovery, a selection declares original-raw or eligible-derived. Unchanged originals retain their real selected raw attempt, digest and path. Eligible derived selections retain the true nonconforming raw invocation/assignment/attempt, digest and location **separately** from selected repaired bytes, digest, location and derivation. A recovery join identifies actual new worker executions and retains the original failed/cancelled status; it is not another reviewer judgment or a fabricated successful raw attempt. Fidelity and availability checks plus driver triage are mandatory.


**External-work successor qualification:** LE-170/177 replace new evidence dependence on same-worker retry, engine invocation/assignment selection and captured packets with actual supplied requests/instructions, original returns and faithful separately attributed driver-confirmed projection. Preserve known identity/content checks, original result/author/findings/rationale, visible gaps and inert candidate previews. Valid siblings survive malformed siblings; missing substantive meaning is not formatting. No engine-executed claim or new work registry is created.

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
- Status: live
- Coverage: e2e/journey

For unchanged original-raw selection, real selected attempt/digest/path and mechanically knowable result, findings, author, gate/stage/configuration and current target must still agree at append and checked evaluation, including manual source-linked append. Only eligible LE-151 derived selection may differ from a true nonconforming raw attempt: independently verify that raw origin and the separately selected derived bytes/digest/path/derivation and driver-triaged fidelity, and compare source-linked records to the **selected derived bytes**, not falsely to the nonconforming raw bytes. Do not invent an attempt, infer external author authenticity or call formatting an independent pass.

**Stable-reference delivery note:** The caller now supplies only a same-run invocation/assignment origin reference. Core resolves the selected attempt, digest, path, capture, command, and binding from durable engine state; the provider performs the existing byte and judgment-field checks. This delivery does not add driver-authored provenance duplication under LE-107.


**External-work successor qualification:** LE-170/177 replace new evidence dependence on same-worker retry, engine invocation/assignment selection and captured packets with actual supplied requests/instructions, original returns and faithful separately attributed driver-confirmed projection. Preserve known identity/content checks, original result/author/findings/rationale, visible gaps and inert candidate previews. Valid siblings survive malformed siblings; missing substantive meaning is not formatting. No engine-executed claim or new work registry is created.

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
- Status: live
- Coverage: e2e/journey


**External-work successor qualification:** LE-170/177 replace new evidence dependence on same-worker retry, engine invocation/assignment selection and captured packets with actual supplied requests/instructions, original returns and faithful separately attributed driver-confirmed projection. Preserve known identity/content checks, original result/author/findings/rationale, visible gaps and inert candidate previews. Valid siblings survive malformed siblings; missing substantive meaning is not formatting. No engine-executed claim or new work registry is created.

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
- Status: live
- Coverage: e2e/journey

A shared provider selector supports unbound commission inspection, opaque bound context filtering and exact plan-task packets. User-steering names all, slots, or current-plan tasks; explicit whole-record supersession and append order replace inferred merging. Unknown recipients/references refuse, stale task revisions are visible and not applied, and task instructions do not spread to dependants or summarizer. A launched attempt retains its original selection. Execution owner/argv updates name existing proof obligations and a reason without revising unchanged outcomes, decomposition or proof. Unbound incorporation records are testimony, not proof of obedience; public proof must show delivered steering changing work.


**External-work successor qualification:** LE-167/168/171/178 retire executor-specific skipped-bound checks, owned-work barriers and launched task/summarizer packet requirements, not actual steering, exact-source dispositions, visible exceptional history or unresolved-findings refusal. Count eligible distinct actors under selected policy; explicitly permitted self-review does not override configured independence/cold eligibility. A count amendment neither discharges a failure nor converts an exception into normal completion.

### LE-115: Reasoned exact-source finding dispositions discharge failures without rewriting judgments or weakening author floors.
- Status: live
- Coverage: e2e/journey

Software-change counts distinct current independent non-retired judgments: a pass or a fail explicitly rejected/resolved by its exact evidence identity. Discharged failures are satisfied-by-disposition, not passes. Undispositioned fails and accepted-unresolved findings block with source/author/remedy diagnostics; a revision bump does not resolve the latter. Historical resolved sources/routing remain inspectable without false applicability. Reasoned retired-author disposition requires recorded gate roster change showing departure and replacement coverage; retired authors do not count. Source/capture mismatches remain mechanical denials; semantic disposition belongs to the driver.


**External-work successor qualification:** LE-167/168/171/178 retire executor-specific skipped-bound checks, owned-work barriers and launched task/summarizer packet requirements, not actual steering, exact-source dispositions, visible exceptional history or unresolved-findings refusal. Count eligible distinct actors under selected policy; explicitly permitted self-review does not override configured independence/cold eligibility. A count amendment neither discharges a failure nor converts an exception into normal completion.

### LE-116: Final software-change validation indexes complete independent current-criterion and whole-intent evidence at one checkpoint.
- Status: live
- Coverage: e2e/journey

Supporting software-change contracts use current frozen AC-N identities and independent criterion_policy, separate from review-axis author counts. The accepted plan names runnable proof_commands and owners; command captures retain actual argv/cwd, exit, elapsed time, output and repository identity. Failed, missing or incomplete proof cannot pass. The fixed report indexes command evidence, exactly one selected verdict set per criterion and a separate goal judgment; omissions, duplicates, unknown/stale/self-authored/unsupported evidence refuse. Prechosen unused record IDs are only names: checkpoint the index before genuine append --record-id judgments, with no placeholders or reservations. Validation-ready may leave verdicts pending for live review; approval or reviewless draft-to-end requires completeness. Ordinary review uses the retained command collection; challenge consumes the completed criterion/goal collection without recommissioning it. In high ordinary validation, individual-axis workers return axes only, while the aggregate authors alone produce their criterion/goal rows and all-axis judgments together. Pending verdict IDs contain no evidence. Approval requires the actual complete current collection. Minimal/standard retain their ordinary combined-output pattern. After repair name affected criteria and supply fresh verdicts; explicitly carry unaffected original evidence to the current report/checkpoint with driver/reason and visible original author/result. Material repair requires fresh goal judgment; only explained report-index-only correction may carry it. Unresolved criterion failures block under exact-source dispositions. Normal validation retains accepted implementation-proof-history for the same tree. Explicit driver-added `validation-command` records may strengthen the effective command collection using new distinct IDs; they cannot replace frozen required IDs, waive proof or create acceptance criteria. `proof_updates` remains execution correction for existing IDs, not an addition path. The complete index includes declared supplemental commands and their real selected execution evidence; missing, failed, stale or incomplete evidence is not passing proof. Validation preparation is inert: it may reuse applicable retained execution to prepare command-evidence candidates, an index draft and independent criterion/goal commissions, but does not execute proof, append evidence, checkpoint, issue judgments or progress the run. Repository report receipts and native provider/checkpoint identities remain distinct and are checked under their existing contracts. The driver inspects and finalizes/checkpoints the index before commissioning the required independent judgments.


**External-work successor qualification:** LE-168/171/178/181/183 qualify engine-capture-only and one-repository command acquisition, unconditional subject/report/implementation-author exclusion, fixed floors and categorical fresh aggregate/whole-goal replay. Retain the complete fixed AC/goal index, actual required command obligations, genuine output/assertions, current report/checkpoint, explicit affected checks and independent defaults. Eligible self counts only where explicitly permitted; floors resolve from authorized amendments. Complete-scope applicable goal/aggregate evidence may stand; materially affected judging meaning requires fresh assessment. Missing, failed, unresolved or wrong-source evidence never passes.

### LE-117: Focused workers and bounded isolated proof jobs preserve complete final proof without duplicated suite ownership.
- Status: live
- Coverage: e2e/journey

Workers run assigned focused checks; one designated proof owner runs the complete serialized final stable-tree matrix and repeats only invalidated checks. Reviewers consume retained results rather than rerunning suites. The public journey uses one explicit jobs budget (default two, serial one) only for independent isolated work, preserves dependent run ordering and full inventory, prebuilds binaries, uses private targets for actual compiling jobs, propagates failure and verifies descendant cleanup. Repeated comparable measurements must show targeted lower wall time, with clippy/source journeys and available launch/retry/token/cost observations reported separately, never an invented baseline or universal speedup. Hosted exact-commit proof and later real dogfood remain pending until observed.


**External-work successor qualification:** LE-167/176 retire engine/provider proof jobs, executor cleanup and dispatch prerequisites, not one designated stable-tree proof owner, focused worker checks, retained outcomes, failure visibility or required workspace/public/package proof. This external-work amendment makes no quantitative speedup claim and does not adopt unrelated pending recovery measurement promises. Actual required cache statistics and wall time remain reported; a separately accepted benchmark needs its exact inputs/argv and current results.

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
- Status: live
- Coverage: e2e/journey

All shipped levels include ordinary and challenge review at intent, design, plan, implementation and validation, using the full axis sets specified by the software-change PRD. Minimal requires one independent author covering every axis together. Standard requires two independent authors, each covering every axis together.

High requires two independent authors each to review every axis individually, followed by those same two identities each reviewing all axes together on the unchanged subject before fixes. First aggregate reviews use fresh sessions and receive neither author's individual-stage findings or judgments. Both stages remain visible and separately required. After fixes, only affected individual axes need fresh review; unaffected coverage requires explicit applicability. Both aggregate authors then freshly review every axis.

Every assigned axis appears exactly once in its commissioned output. Mixed pass/fail output is valid. Missing, duplicate or unknown axes cannot satisfy review. The existing one same-worker conformance correction preserves both raw attempts. Candidate projection retains the real invocation/assignment origin and labels carried references separately. Exact-source dispositions remain subject to LE-115; accepted unresolved findings block across revisions. No stage can satisfy the other stage's obligation.

Final criterion and whole-goal author counts are one for minimal and two for standard/high, independently of axis batching. High's extra stage creates no additional author floor or proof-suite execution. Shipped profiles remain unbound and contain no selected model or machine-local command. Bookends remains off until explicitly enabled for a run. This replaces LE-118's universal single-commission default.


**External-work successor qualification:** LE-168/171/173/175/178/182 preserve default phase/axis coverage, independent floors and configured individual/aggregate author relationships, while making groups, permitted self and stages explicit pre-start policy. Add the separate intent-sufficient singleton and broad acceptance-sufficient question at existing reader floors. Counts may change only through the authorized count exception. Actual coupled author sets must match; unequal minima are not inherently inconsistent. Original prose/YAML/JSON need not obey a provider schema or exact call count. A configured combined perspective still needs an actual combined assessment. Applicable complete-scope aggregate/goal judgments need not replay after unaffected corrections. Ordinary group clearance precedes corresponding challenge.

### LE-131: Each software-change implementation task names the current acceptance criteria it serves.
- Status: live
- Coverage: e2e/journey

New supporting profiles require a nonempty set of current criterion references on every implementation task. Missing, duplicate or unknown references fail deterministic validation. Reviewers judge whether the links meaningfully cover the task's work; unrelated links cannot establish coverage. Other intermediate links remain optional. Final validation still covers every current criterion and the whole goal. Task references introduce no second requirement-ID system.

### LE-132: A small explicit worker roster produces inspectable per-run software-change setup.
- Status: live
- Coverage: e2e/journey

An operator selects a bundled rigor example **or a copied complete standalone profile file**, and supplies worker commands and arguments, including model and effort arguments where applicable. A deterministic setup utility assembles independent author assignments, review stages and optional implementation binding from the selected file's actual supported contents, not its basename or a substituted embedded default. Any command meeting the worker input/output contract is eligible. Invalid, incomplete or insufficient input fails without reducing review obligations.

The resulting exact effective policy, bindings, selected-file bytes/hash, Bookends/advice enablement and commands are visible for owner confirmation before start; inconsistent coverage or an already wrapped task-worker input refuses. Started policy, stages and author floors stay frozen; amend-binding changes only future execution, and changing a floor requires a new run. Setup does not start a run, select models, adapt incompatible worker CLIs, save implicit preferences, track default updates or manage worktrees. Shipped data works through both source and packaged interfaces. Semantic provider operations retain their existing work and judgment boundary.


**External-work successor qualification:** LE-167/168/174/178/180 retire executable-roster/binding construction and the unconditional new-run requirement for count changes. Keep deterministic inspectable preparation, full custom-file authority, actual base/override/effective identity, no model choice/start/judgment or checkout ownership. Common counts/self/groups/axes/stages/advice/Bookends knobs have a small base-derived path; raw schema/prompt replacement may remain full-custom. Future preparation explicitly selects a newer base with visible drift/conflicts; active semantic policy never silently tracks defaults.

### LE-133: Bookends checks every newly published reachable requirement transition.
- Status: live
- Coverage: e2e/journey

Pre-push and required CI inspect every commit introduced by each updated ref, including merged branch commits, and check continuity against each relevant parent. Per-commit continuity (no silent removal, reassignment, revival, adoption removal, or tombstone games) still blocks wherever it appears, and incomplete history still fails closed. Coverage-timing findings on intermediate commits (eligible citations landing later in the same pushed range) are retained as visible diagnostics instead of blocking, provided the pushed tip tree is independently clean. Legitimate first adoption and permanent tombstones retain their meaning. Coverage of the published tip is checked against that tip's content. Known residual: landing an uncovered live ID then tombstoning it in the same range passes; tombstones are permanent and review-visible.

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


**External-work successor qualification:** LE-167/168/174/178/180 retire executable-roster/binding construction and the unconditional new-run requirement for count changes. Keep deterministic inspectable preparation, full custom-file authority, actual base/override/effective identity, no model choice/start/judgment or checkout ownership. Common counts/self/groups/axes/stages/advice/Bookends knobs have a small base-derived path; raw schema/prompt replacement may remain full-custom. Future preparation explicitly selects a newer base with visible drift/conflicts; active semantic policy never silently tracks defaults.

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
- Status: live
- Coverage: e2e/journey

For each future new minimal, standard, and high-rigor software-change run, ordinary intent review and the separate challenge review must expose acceptance-granularity as an externally judged question while retaining each profile's declared stages and required number of independent reviewers. The question assesses whether each criterion is a coherent bounded outcome, whether independently varying outcomes are bundled, whether inseparable parts have been split needlessly, and whether evidence can be obtained in practice. A material failure names the exact criteria and its consequence for delivery or acceptance. Word counts, conjunction counts, fixed criterion counts, and review ceremony do not replace judgment. Shipped setup, guidance, policy data, and applicable real end-to-end workflow checks expose the same question. The provider validates evidence shape and does not perform semantic review or claim calibration. Frozen runs keep their original obligations.


**External-work successor qualification:** The granularity/comprehensibility judging questions remain unchanged. LE-168/178 qualify unconditional author exclusion and fixed-count wording only for explicit pre-start eligible-self permission and authorized count-only amendments. Independent and cold obligations remain binding, each actor counts once, and no word/length/jargon/count proxy becomes a materiality norm.

### LE-142: Proposed software-change reconciliation state and semantic coverage
- Status: live
- Coverage: e2e/journey

For each promised enduring software-change intent outcome in an ordinary new software-change run, a supported reconciliation state after implementation edits and before final repository proof/checkpoint and implementation review must inspect the outcome against the approved intent and current authoritative documents. In Bookends-enabled runs, it must inspect the actual accepted requirement text and every authoritative document that the requirement explicitly names, then classify the outcome into one of three branches: (1) sufficient existing accepted wording, which requires only the needed implementation and public proof while retaining honest live traceability; (2) change-specific proof, which creates no permanent requirement and requires no proposal or PRD commit; or (3) missing or changed enduring requirement meaning, which requires exact proposed wording in the correct owning document. Only the third branch requires exact owner acceptance, separately authorized application and commit, updated live traceability, and independent semantic inspection of the actual PRD text and associated public proof before completion. A related ID, shared topic, or token match is insufficient. A justified no-document-change result is valid. An unresolved discrepancy, missing required owner authorization, necessary proposal that is unaccepted, unapplied, or uncommitted, or a merely related citation blocks completion. When delivered behavior is wrong, the implementation is corrected; when requirement meaning is missing or changed, the exact owner-accepted amendment remains unapplied until separate Git authorization permits its application. Bookends-disabled runs reconcile relevant repository documentation against the approved intent and delivered behavior without PRD IDs, Bookends machinery, or overlay obligations. Public journeys inspect resulting document contents and progression for successful edit, justified no-change, correction or blocking, and both Bookends modes; state existence or synthetic attestations alone is insufficient. Frozen runs retain their existing graph and evidence. This adds no second requirement-ID spine, per-criterion ledger, automatic semantic matcher, extra review axis, engine-level workflow mechanism, or migration.

### LE-143: Proposed owner-facing proactive software-change updates
- Status: live
- Coverage: e2e/journey

For active software-change work, the coordinating assistant must read existing public status or monitor output and post a concise source-backed owner-facing update in the active Pi conversation after a meaningful development, blocker, outcome, or owner decision is observed and before the first subsequent driver decision point to wait, inspect, or seek owner help. Unchanged observations must produce no repetitive heartbeat messages. Machine-consumable completion or attention notifications alone do not satisfy the owner-facing communication obligation. Source-to-observation lag and repeated notification after observer restart remain permitted. The communication stays passive and does not approve, advance, retry, cancel, or route work, and it imposes no guaranteed real-time deadline or new channel.

### LE-144: Proposed owner-comprehensible software-change intent obligation
- Status: live
- Coverage: e2e/journey

For future new software-change runs, the single authoritative intent must let its intended owner understand the problem, outcomes, scope boundaries, and acceptance decisions from the intent and its stated background. Necessary specialist terms must be explained when they first matter. Exact technical references and structured metadata remain alongside the plain-English decision content, preserving obligations, uncertainty, qualifications, and precision. Ordinary intent review and the separate challenge review must expose owner-comprehensible as a distinct externally judged question while retaining each profile's existing stages and required number of independent reviewers. The question fails only when specific wording or an unstated dependency materially blocks understanding or evaluation; the finding names the affected obligation and likely misunderstanding. Style preference, sentence length, reading-grade score, and technical terminology alone are insufficient for failure. Shipped drafting guidance, profile data, review prompts, and applicable checks use the same question identity. Frozen runs retain their original obligations.


**External-work successor qualification:** The granularity/comprehensibility judging questions remain unchanged. LE-168/178 qualify unconditional author exclusion and fixed-count wording only for explicit pre-start eligible-self permission and authorized count-only amendments. Independent and cold obligations remain binding, each actor counts once, and no word/length/jargon/count proxy becomes a materiality norm.

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


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-149: Review-necessary context and driver-owned fit
- Status: live
- Coverage: e2e/journey

A bound reviewer receives exact duties, target, relevant frozen constraints and current findings/applicability with locators and accessible necessary originals, not unrelated accumulated history. First ordinary/first high-rigor aggregate input excludes competitors' same-target judgments and both individual authors' findings, including indirect ledger/applicability leaks; confirmation includes findings it must check, and challenge includes actual parent grounds. Preambles describe only what was delivered. For new software-change runs, the engine and provider steward compact relevant review material without arbitrary review-content byte/token ceilings or mandatory per-model window/reserve declarations, accounting or capacity refusal. The driver may select or filter material for a particular run while retaining accessible necessary originals, meaningful durable history and truthful capture of what was delivered; filtering cannot silently truncate necessary facts, split a required obligation, substitute a model or waive review. Repeatable public operator-path proof includes representative detailed current and materially revised intents with retained owner sources and exact prior-intent baselines. Real-model checks are additional only where deterministic proof cannot answer the relevant question; no universal model-fit or semantic-quality claim follows. This changes review-content responsibility only, not model-spend, subprocess, cancellation or transport safety limits. Existing frozen runs retain their original obligations and runtime.


**External-work successor qualification:** LE-167/168/170/176/177 preserve passive provider-neutral workflow/evidence inspection, bounded truthful context, necessary sources, first-assessment peer isolation and actual challenge grounds. Retire live worker/Dagu/capture-matrix sampling, execution labels and mandatory bound fan-out/graph journey assertions. Externally owned execution/liveness remains unknown unless explicitly observed; observer output has no progression authority. Public proof completes meaningful externally driven workflows and tests changed provider-specific boundaries, not obsolete production executors.

### LE-150: Applicable work survives same-run plan reshaping
- Status: live
- Coverage: e2e/journey

Within one run a driver may explicitly map original fulfilled obligations to revised tasks, including rename/split/merge or changed intent/design, after checking real effects, dependencies, current tree and proof. Reuse only verifiably applicable results and eligible evidence; uncertain, affected or missing prerequisites remain pending and their dependants rerun. Neither matching names nor unchanged files establishes standing. Preserve original records and failed attempts, refresh affected proof and post-reconciliation report/checkpoint on the actual final tree, and obtain required fresh independent review. No cross-run transfer or automatic semantic matcher follows.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-151: Attributed output-only recovery without new judgments
- Status: live
- Coverage: e2e/journey

For a representation/schema-only failure after explicit substantive review or implementation output, a driver may directly supply a meaning-preserving corrected representation through the existing output-only recovery path without another model call or whole-round reviewer rerun. Unambiguous mechanical correction and an optionally configured representation-only repair model remain alternatives, not prerequisites to direct driver correction. A repair restores constants or reformats only explicit original meaning: it never invents rationale, findings or verdict, changes repository bytes, reruns implementation commands or silently substitutes a model. Preserve the original reviewer identity, result, findings and rationale; retain immutable raw attempts/errors alongside separately attributed selected derived bytes, difference, true raw origin and driver fidelity triage. Conformance diagnostics identify the actual defective field and schema location, including the underlying cause of nested alternative failures, or the specific representation error. Missing, contradictory or ambiguous substantive meaning needs original-author clarification or stays unsatisfied. An optional same-author retry is a new substantive judgment, not a formatting label. Only eligible derived output with no conforming selected original raw output can enter source-linked admission under LE-99/105; formatting alone proves no gate. A configured model alternative retains owner-chosen identity and enforceable positive call, elapsed and metered-cost limits; failed, exhausted or unmetered execution refuses without fallback. Direct driver correction does not fabricate adapter usage or another independent reviewer. Supporting new profiles explicitly opt into the changed recovery contract; existing frozen runs retain their original obligations and runtime.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-152: Explicit partial-invocation recovery
- Status: tombstone

After failed or cancelled fan-out, expose stable IDs for verifiably complete siblings, invalid/incomplete members and never-started downstream members. Following verified owned-work quiescence, the driver explicitly selects necessary reruns or a genuine same-binding mechanical recovery join. Joined selected sources retain their failed/cancelled raw origins, source class and original captures while the old invocation stays failed/cancelled; a new succeeded outer invocation is never represented as new reviewer judgment. Real barriers, selected bytes and current applicability still govern checked admission and required fresh review. A substantive reviewer fail is not an execution retry; no automatic whole-gate restart or individual-worker cancellation is required.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-153: Honest narrow driver-owned correction and same-graph final report
- Status: tombstone

Only a graph explicitly opting a **non-review draft or implement slot** in may accept a small understood driver-authored code or workflow-artifact correction when accepted intent, design and decomposition are unchanged and no conflicting work owns it. Record current visit, binding, author, changed work and reason as a distinct non-worker pre-evaluation completion act after quiescence; still run the normal checked provider/artifact/evidence/Bookends gate and refresh affected proof and independent reviews. Never fabricate bound success, edit raw attempts, perform reviewer work by driver act or use the general override as ordinary repair. Larger or uncertain changes return to their owner/phase. After an actual LE-142 decision and authorized document changes, a separate future report-only selection on the **same bound plan graph**, if all tasks still stand, runs only its frozen summarizer and generates a fresh current-tree report/checkpoint before re-entry; affected real tasks instead use supported task-root selection. Neither selector migrates an existing frozen run, authorizes Git, or labels a pre-decision report final. Where no supported report path and no genuine affected task work exist, stop and escalate rather than manufacture task work.


Retired by LE-167/176 for the external-work successor. The exact original title and earlier accepted/proposed history are retained; this retirement does not adopt a pending proposal or preserve an executor.

### LE-154: Invalid reuse cannot erase valid fresh siblings
- Status: live
- Coverage: e2e/journey

A confirmation reuse row must name applicability actually delivered to that assignment and the exact original author, axis, stage, gate and current target. Wrong-kind, wrong-gate, stale or retroactive references receive an early row-specific diagnostic, count neither as a pass nor a carried judgment and leave that obligation/gate unsatisfied. After the whole-batch author, axis set, stage, schema and selected-byte envelope is intact, independent structurally valid fresh sibling rows may still be projected and source-verified/admitted; an invalid reuse row cannot erase them or make the whole gate pass. Ambiguous identity or content is never guessed; raw output is unchanged.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-155: Grounded passes and challengeable reasons
- Status: live
- Coverage: e2e/journey

Each software-change review-axis and independent criterion/whole-goal pass carries a relevant reason and references to actual inspected evidence. New software-change review contracts impose no arbitrary rationale-length or evidence-count/locator-length quotas and do not require reviewers to author source-file hashes when existing durable references, captures, target freshness and driver judgment suffice. Mechanical source checks reject malformed, wrong or unavailable references without asserting semantic support or exact historical arbitrary-file identity. Retain existing engine-owned raw/selected capture integrity and current-subject/checkpoint checks; do not replace the removed reviewer burden with hidden digest sidecars, a new provenance inventory or copied mechanical metadata. The driver investigates weak grounds proportionately and attributes any corroboration, clarification, faithful representation correction or re-review separately. It cannot change an original judgment's meaning or claim another independent reviewer. First peers remain independent; challenge receives the parent's actual grounds. Real good and defective cases, including false but mechanically valid citations, are needed; no compulsory essay or new review axis is added. Existing frozen runs retain their original output contract and evidence meaning.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-156: Source-faithful intent and truthful approval
- Status: live
- Coverage: e2e/journey

For initial and every materially revised software-change intent, a reviewer other than the drafter compares the full draft to retained qualified owner statements and their supersession, not driver inference presented as owner instruction. The driver triages differences and returns consequential unresolved meaning to the owner. Before approval the owner sees the single full intent, boundaries, open decisions and exact substantive wording delta where prior bytes exist; drafter/reviewer/driver work and the owner's own acceptance remain distinct. Do not add a second final owner-source approval at delivery.

### LE-157: Merit-based finding resolution and visible exception
- Status: live
- Coverage: e2e/journey

A software-change driver may reject an evidenced factual mistake directly, but an unresolved substantive dispute about a material in-scope finding receives focused **independent** reconsideration against the original source. Rejection must rest on evidence, scope or materiality, never re-entry cost, schedule or an earlier parent pass. If a real material defect exceeds authorized budget or scope, present the consequence and repair choices to the owner. Only a specific owner-attested scoped exception may leave the defect and its original history visible with an exceptional rather than ordinary completed outcome; neither the provider nor an advisor judges the truth of a reason or rewrites a reviewer's judgment. Existing no-waiver, independent-author and exact-source obligations remain; no extra review axis or model is imposed.

### LE-158: Preview and exact apply of factual evidence
- Status: live
- Coverage: e2e/journey

From selected authoritative source outputs, software-change offers a nonmutating exact current-target preview for review and criterion/goal record candidates: real result/findings, author, gate/stage/configuration, revision/checkpoint and stable origin. A driver explicitly triages and applies them through ordinary observed append; a factual-only optional finding-ledger skeleton copies verbatim source facts and prior IDs/status, not reason, disposition or routing. Interrupted apply skips only an identical earlier ID, bytes, source and target; conflict or drift refuses. Preview never satisfies a gate or creates another ledger.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-159: Per-criterion distinguishing plan-time proof
- Status: live
- Coverage: e2e/journey

**In addition to** LE-97's observable operator path/black-box-or-impracticality rule and LE-116/134's final current-index/public-assertion requirements, before plan approval each current AC-N names the actual operator-path observation and assertion that would differ on failure. Name a runnable current-tree negative or practical before-fix execution with setup, expected discrimination and paired named current proof; execute practical cases and retain genuine historical identity/output separately from the passing current-tree index, or give a case-specific concrete impracticality and nearest feasible substitute. Final current command captures and independent verdicts cite the specific distinguishing assertion for each AC, even when several share one command. Do not add a second criterion ledger, turn old expected failures into current passing evidence, or rerun unaffected suites for ceremony.


**External-work successor qualification:** LE-169/170/171/177/181/183 replace dependencies on scheduled dependent reruns, captured-commission delivery, recovery joins, selected raw invocations and repair-model dispatch with supported faithful external-source recording, correction and scope-sensitive applicability. Preserve meaningful task dependencies, exact original judgments/attribution, valid siblings, grounded reasons/accessibility, inert previews and distinguishing criterion assertions. Recording or a valid locator alone is not semantic proof. Current external command results and source scopes retain actual producer/identity; no missing judgment or proof is invented.

### LE-160: Standalone editable profile examples
- Status: live
- Coverage: e2e/journey

Minimal, standard and high examples can be exported and copied as complete standalone files for future setup/launch. The selected file's supported policies, stages, counts, schemas and commands—not its basename or embedded defaults—determine the effective preview and run. Refuse inconsistent coverage and pre-wrapped task-worker input before start; show exact effective bytes, hash and bindings. Started policy and floors remain frozen; binding amendments affect future execution only, while changing frozen floors requires a new run. No inheritance, editor, preference store or automatic default tracking is required.


**External-work successor qualification:** LE-167/168/174/178/180 retire executable-roster/binding construction and the unconditional new-run requirement for count changes. Keep deterministic inspectable preparation, full custom-file authority, actual base/override/effective identity, no model choice/start/judgment or checkout ownership. Common counts/self/groups/axes/stages/advice/Bookends knobs have a small base-derived path; raw schema/prompt replacement may remain full-custom. Future preparation explicitly selects a newer base with visible drift/conflicts; active semantic policy never silently tracks defaults.

### LE-161: Phase-appropriate five-artifact Bookends coverage
- Status: live
- Coverage: e2e/journey

When Bookends is enabled, intent, design, plan, implementation report and validation report each meaningfully explain its phase-appropriate coverage of all accepted in-scope live requirements and applicable obligations in documents those requirements explicitly name. Reconcile omissions without silently ignoring them or broadening scope. Related IDs, parser-valid candidates, a list pasted onto every task or passing commands do not establish coverage; reuse the AC-N/plan/fixed proof spine. Final eligible public assertions and real Bookends GREEN remain mandatory. Bookends-off artifacts acquire no PRD-ID duties.

### LE-162: Early Bookends prerequisite visibility
- Status: live
- Coverage: e2e/journey

Before plan approval in Bookends mode, a read-only check exposes missing repository config, actual accepted/live requirement wording and explicit authoritative cross-references, eligible public proof locations, recognized required CI collection and evaluation cwd, with actionable work planned for missing prerequisites. It does not claim semantic sufficiency or require behavior still being implemented to pass. Final public eligibility and GREEN are unchanged.

### LE-163: Generic optional typed advisory invocation
- Status: live
- Coverage: e2e/journey

A configured provider-reusable command receives one structured JSON state and independently keyed questions, with exact-key typed answers and separately retained original input, command, response and failure; no backend or model is selected by the engine, and advice cannot complete primary work or grant workflow authority. Choice supplies selected named option key, finite normalized full probabilities over exactly the supplied options and confidence in [0,1]. Score supplies a probability-weighted value over ordered rubric-level indices, a matching indexed level-to-description legend, finite normalized full level probabilities and confidence in [0,1]. Noul supplies a yes-probability in [0,1], **without mandatory separate confidence**. Each type permits absent or present per-answer string rationale. No fourth abstain type, invented missing probability/field, required prose, vendor HTTP envelope or named backend follows. A generic non-software provider can use the same interface.

Every advisory question must be one bounded judgment over sufficient explicitly supplied evidence. Independent questions may be batched; questions requiring investigation, missing-fact inference, planning, multi-step reasoning, or whole-workflow synthesis must not be sent to an advisor. Those tasks remain with the coordinating driver; there is no automatic reasoning-model fallback through the advice feature.

The driver is responsible for deciding whether a question and its evidence meet that boundary before invocation. Mechanical checks may reject missing or malformed inputs, but must not claim to prove semantic sufficiency. Shipped question builders must not generate the excluded classes of question.

Advice remains fallible and non-authoritative regardless of confidence. The driver accepts, partially accepts, or rejects each actual answer with a reason.


**External-work successor qualification:** LE-167/172/176 remove advisor backend command/configuration, invocation and attempt dispatch, not configured substantive typed questions, choice/full probabilities/confidence, score/legend/full probabilities/confidence and Noul probability with optional confidence/rationale. Genuine externally produced answers and actual accept/partial/reject dispositions close configured occasions; a permitted no-admissible-question reason stays explicit. Missing required values cannot be projected into existence. Normal checked/check-free departure closure and visible scoped exception/refusal remain; advice is neither review approval nor proof/route authority.

### LE-164: Software-change advice for eight evidenced decisions
- Status: live
- Coverage: e2e/journey

Software-change supplies provider-owned questions and actual selected evidence for finding support/materiality/scope separately, correction owner, implementation route including only authorized narrow driver repair, execution recovery or owner escalation, semantic evidence applicability, three-way requirements reconciliation, review-round convergence and each current AC-N plus whole-goal/check-gap fulfillment. Batch independent questions; dependent ones wait for real driver triage or fixes. Advice never discovers repository defects by fiat, decides a route, replaces independent reviewers or weakens finding-ledger, Bookends or public proof obligations.

These occasions identify where bounded judgments can assist the driver; they do not assign the broader decision or synthesis to the advisor. Preserve both agent-supplied claims and tool-resolved selected evidence rather than replacing the latter with an agent paraphrase.


**External-work successor qualification:** LE-167/172/176 remove advisor backend command/configuration, invocation and attempt dispatch, not configured substantive typed questions, choice/full probabilities/confidence, score/legend/full probabilities/confidence and Noul probability with optional confidence/rationale. Genuine externally produced answers and actual accept/partial/reject dispositions close configured occasions; a permitted no-admissible-question reason stays explicit. Missing required values cannot be projected into existence. Normal checked/check-free departure closure and visible scoped exception/refusal remain; advice is neither review approval nor proof/route authority.

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
- Status: live
- Coverage: e2e/journey

Users can configure review assignments containing one or multiple axes, with individual, aggregate and blindness expectations where applicable.

Providers supply editable requests for the configured assignments and check declared coverage, stage and authorship obligations without requiring exact prompts, commands or agent-call counts. Review-group membership, required perspectives, stages and author-eligibility obligations are frozen run policy, not dispatch suggestions. Required review-author counts may change only through the explicit durable amendment in LE-178. During the run, the driver may adapt request wording, relevant context and harness packaging that preserve those obligations, but may not silently regroup axes or weaken a configured review perspective. Other changes to that semantic policy require an explicitly selected new profile/run, not an execution amendment. The author-count exception does not authorize silent regrouping, changed judging questions, stages, independence, blindness or eligibility. Blind reviews receive necessary requirements and evidence without peer conclusions.

Shipped default profiles do not enable self-review for their obligations. They preserve independent review by default. The user can explicitly permit self-review for chosen assignments, acceptance criteria and the whole goal before start through supported guided configuration; no hand-editing of a low-level custom profile is required. An eligible subject/report/implementation author counts toward that obligation's distinct eligible-author floor: for example, an explicitly self-review-permitting two-author obligation can be satisfied by its author plus one other eligible author. Permission does not lower the floor or count one actor twice. An explicitly independent or cold-reader obligation still excludes self-authorship. A one-author self-review-permitting obligation can be satisfied by its author, honestly labeled; a custom self-only configuration must omit any cold axis it cannot fulfill. Normal completion is possible under the selected policy, but results must not falsely claim independence or blindness. Independent requirements remain binding wherever configured. Shipped phase/review variants, author floors and stage-author relationships retain their current meaning by default, except for the explicit grouping, self-review and proportionate-reuse changes described here.

### LE-169: Useful plans without mandatory agent isolation
- Status: live
- Coverage: e2e/journey

Plans retain schema-checked tasks, criterion coverage, intended outcomes and meaningful dependencies.

The driver can perform work directly, combine tasks into commissions, split commissions or develop related tasks together. Dispatch changes do not require plan revision; material changes to intended work or dependency meaning do.

Neither one-agent-per-task execution nor a designated graph summarizer is required.

### LE-170: Reliable guided recording and correction
- Status: live
- Coverage: e2e/journey

Documented, supported recording paths use known assignment/workflow information to reduce manual metadata work while leaving semantic judgments with the driver. External returns may be free-form prose, YAML or structured/fenced JSON; the reviewer is not required to emit a provider-specific schema. Actual prompt/request and return content may live in evidence-record fields or referenced retained content; no separate-file storage layout is prescribed. The guided path supports faithful driver-confirmed judgment projection and known metadata completion, not automatic semantic understanding of arbitrary prose.

Retain the actual driver-supplied commissioning prompt or request and original return. For direct work or self-review, retain the instructions actually followed and the original resulting judgment; do not invent a separate dispatch or agent response. Faithful representation corrections preserve verdicts, findings, rationale and attribution. They do not invent missing meaning, discard valid siblings or require new model calls solely for formatting.

Diagnostics distinguish recording problems from missing substantive evidence and explain the correction. Supported guidance handles clear grouped multi-axis prose, YAML/fenced JSON, direct/self-review recording, and a malformed sibling alongside valid judgments without bespoke rescue scripts, repeated hand-copying of machine metadata, or formatting-only reviewer calls.

### LE-171: Ordinary, proportionate reuse
- Status: live
- Coverage: e2e/journey

Genuine findings, review judgments, implementation work and command proof can remain applicable across revisions and corrections within the same run through an explicit, guided applicability path. Cross-run reuse/import is outside this delivery.

Original identities and results remain truthful. The driver explains applicability and obtains affected checks or judgments. This includes aggregate and whole-goal judgments when their complete scope and judging meaning remain applicable; existing categorical aggregate/goal freshness requirements are replaced by applicability-sensitive requirements. Materially affected judgments require fresh assessment, and stage/author coverage remains binding. Unresolved findings remain visible; reuse cannot silently discharge them or manufacture passing evidence.

### LE-172: External advice with preserved configured obligations
- Status: live
- Coverage: e2e/journey

Providers supply proposed advice requests that the driver can inspect and adapt; neither engine nor provider dispatches the advisor.

When advice is enabled, the driver performs the configured external step—or records why no admissible question exists—and dispositions genuine responses before departure. Preserve the existing typed substantive question/answer semantics, including configured choice, score and probability requirements. Requests and original returns may use free-form or structured representations, but a qualitative answer lacking required values cannot fulfill a typed obligation and guided recording cannot invent those values. LE-170's representation flexibility is not permission to redefine the advisor's job. Advice alone is not approval or a substitute for required review and proof.

### LE-173: Intent sufficiently constrains the intended final state
- Status: live
- Coverage: e2e/journey

Every shipped software-change profile enables `intent-sufficient` in ordinary intent review as its own singleton aggregate-stage group, separate from the profile's other intent axes. Use the profile's existing configured independent-reader floor; no additional default individual-stage assignment is required. Its informed challenge likewise has its own corresponding group. Existing broad groups keep their combined perspective. Early brief preparation is specified in LE-174. Qualification of early evidence, including independence from unrelated groups, is specified in LE-177. Even when other obligations allow self-review, this enabled cold exercise needs a genuinely unbriefed reader and cannot be labeled blind self-review.

Declare the pre-change implementation baseline for the initial exercise and retain that same source baseline across revisions of this intent. Renewed readers do not implicitly receive partially implemented code as a replacement baseline. The owner may explicitly select a changed baseline, which requires a fresh cold exercise and comparison; the paired challenge uses that same declared baseline. Use existing durable Git/source references or supplied source material, without requiring a new snapshot or worktree manager.

Given only the current intent and that declared pre-change implementation baseline, the cold reader first describes the anticipated final state, then actively searches for other plausible final states permitted by the same intent and baseline. For each material alternative, identify the wording or missing constraint that permits it and explain the difference in behavior, responsibilities or acceptance obligations. Do not manufacture alternatives, exploit strained readings or introduce requirements outside the stated scope.

Before the ordinary intent gate can pass, retain every current qualifying cold reconstruction counted toward the configured independent-reader floor. The owner's or an explicitly owner-authorized disposition must address all those reconstructions and their useful material alternatives; one collective comparison is sufficient, with no separate ceremony per reader. Do not select only the convenient return and ignore a counted reader's alternative. Distinguish acceptable implementation freedom, unwanted but genuinely permitted alternatives, and unsupported reader assumptions. Clarify missing, incorrect or insufficient constraints in the intent rather than treating private comparison comments as a substitute for the written constraints. No new workflow phase is introduced.

Every shipped software-change profile's existing intent challenge review also includes this axis. This variant receives the current intent, baseline code, retained cold reconstruction and recorded comparison, and tries to falsify the accepted alignment by identifying remaining materially unwanted interpretations genuinely permitted by the written intent. It is not blind to the parent result, requires no second cold readback, and requires no duplicate owner comparison for an unchanged intent. New material mismatches use the normal clarification/revision path.

When an edit materially changes which final states the intent permits—including a clarification excluding a previously permitted material alternative—obtain a new unbriefed complete reconstruction and alternatives test before accepting that revision. Focused assessment by an already briefed reviewer does not replace the cold exercise. Formatting-only or genuinely meaning-preserving edits may retain the original exercise through explicit applicability. Preserve every earlier reconstruction and comparison; neither may be relabeled as a new cold return. Clear prose alone does not establish alignment. Neither original reconstructions nor original judgments may be rewritten to manufacture agreement.

The corresponding challenge tests a cleared alignment conclusion under the ordinary-first sequence in LE-182: all counted reconstructions have been compared, required intent clarifications made and affected ordinary fulfillment confirmed, with no accepted material alternative left unresolved. Early preparation and independence from unrelated groups do not turn an open alignment failure into a qualified challenge.

### LE-174: Review briefs can be prepared early without advancing the workflow
- Status: live
- Coverage: e2e/journey

Across supported providers, the driver can inspect obligations and prepare inert briefs for a selected configured review state or assignment before entering that state, when the required subject and source inputs exist. Missing inputs are explicit. Preparation does not launch work, append judgments, claim approval or change workflow state.

The driver can prepare an intent-sufficient challenge from its actual accepted axis-level alignment claim and recorded comparison before unrelated ordinary obligations finish. This does not claim that the whole ordinary gate has passed. If a multi-axis assignment still lacks necessary parent claims or other inputs, preparation exposes the available context and the missing inputs; it does not claim that the group has been fulfilled.

Default state order and checked departures remain. This preparation capability introduces no new review phase, agent supervisor, mandatory external-work registration or automatic event routing.

### LE-175: Acceptance criteria require the promised result
- Status: live
- Coverage: e2e/journey

Every shipped software-change profile includes `acceptance-sufficient` in ordinary aggregate intent review and its corresponding informed challenge. It joins the existing aggregate review group at the profile's configured author floor; no additional default individual-stage assignment, cold-readback exercise or owner-comparison ceremony is required. The product's inspectable policies, preparation guidance and evidence obligations expose the same ordinary and challenge questions.

The ordinary reviewer identifies each material promised outcome and each specific problem this change claims to remedy, then tests whether a concrete plausible delivery could satisfy all acceptance criteria while leaving that outcome or problem materially unresolved. Assess the named outcomes separately rather than reducing the goal to the selected new capability. Test causal sufficiency as well as visible behavior: the new interaction working does not by itself show that the named cause or burden has been addressed. Explain any concern treated as background rather than a promised remedy.

A failing judgment names the promised outcome, the text-supported counterexample, how it survives every relevant acceptance requirement, and its concrete consequence. Where the intent genuinely leaves mitigation versus elimination unclear, expose that material ambiguity rather than silently assuming the weakest promise. Do not invent goals, require unrelated cleanup or one preferred implementation, demand elimination where only mitigation is promised, or turn legitimate implementation freedom or accepted residuals into defects. An accepted risk cannot waive a stated outcome. No quota or required count of findings applies.

The corresponding challenge consumes the actual ordinary coverage account, verdict, rationale, counterexamples and cited clauses, not a paraphrased or fabricated pass. It attempts to falsify a parent pass with a materially outcome-defeating delivery the criteria still permit, including a named cause that remains outside the new interaction. It also tests whether a dismissed counterexample was genuinely excluded by the current wording, and whether a claimed gap instead invents a goal or ignores a binding criterion. A pass is supported by the relevant clauses ruling out the strongest plausible counterexamples; a fail meets the same outcome/counterexample/consequence burden. This is not a second cold reconstruction, implementation validation, automatic semantic classifier or new workflow phase.

Configured missing or failed evidence remains subject to ordinary finding triage and checked progression; the provider mechanically validates evidence and does not decide causal sufficiency itself. Explicit applicable prior judgments may be retained under the selected policy; new meaningful acceptance constraints require affected review, not relabeled old passes. Shipped examples, operator guidance and required calibration use these exact scoped meanings. Existing A11 genuine fresh/applicable-history rules remain, without treating manual diagnostic returns as complete calibration.

The ordinary-first sequence in LE-182 also governs this axis. Its challenge targets the cleared sufficiency conclusion, not an open ordinary failure. Examining an earlier claimed gap or its dismissal serves the search for a remaining outcome-defeating delivery; it is not a substitute for ordinary triage, correction and confirmation.

### LE-176: Retired agent and proof-command execution machinery no longer imposes a supported maintenance responsibility
- Status: live
- Coverage: e2e/journey

The refactor retires agent execution from the supported engine/provider product, not only its ordinary new-run path. Obsolete agent-execution commands and dedicated implementation, configuration/binding controls, dependencies, and execution-only tests and documentation are removed rather than kept behind a bypass, optional executor or actively supported historical path. This covers invocation launch/waiter lifecycle, agent supervision/cancellation/progress controls, fan-out/plan-runner scheduling and joining, agent-output execution recovery, agent-advice dispatch, and proof-command/matrix launch, execution supervision and cleanup/control surfaces wherever their remaining purpose is that retired responsibility.

Shared facilities with an actual remaining supported use in durable workflow/provider evaluation, evidence recording, faithful external-return handling, or deterministic provider/metadata evaluation and recording supplied command-proof results remain. Shared process, timeout, cancellation, schema, persistence and input/output-recording code is not deleted merely because a retired executor also used it; it must have an actual remaining non-work-execution use. Dependency removal is limited to dependencies with no remaining supported use. Minimal inert historical representations may remain where needed to preserve existing records without continuing old agent execution; historical records and captures are not deleted, reset, migrated or silently reinterpreted.

Acceptance distinguishes retirement from a merely functional bypass: public command/help/configuration surfaces, dependency/runtime prerequisites and source/test/documentation inspection agree that the obsolete agent and primary proof-command execution machinery is no longer supported or maintained. Old-history interpretation remains truthful or explicitly unsupported, and retained provider/workflow/evidence/externally-produced-proof paths still work. No automatic dead-code classifier, exhaustive provenance inventory or unrelated repository cleanup is required.

### LE-177: Qualifying early evidence remains usable when progression reaches its review state
- Status: live
- Coverage: e2e/journey

Early results can later satisfy the corresponding obligation when they genuinely meet the configured group/purpose, stage, eligible authorship, visibility, subject/revision and applicability requirements. Merely entering the formal review state does not invalidate such evidence or require another model call. An ad hoc diagnostic or differently scoped review is not silently promoted into fulfillment of another configured assignment.

A completed grouped judgment must genuinely consider its configured axes together with the required inputs. Isolated axis results are not silently promoted into that combined judgment. Useful partial work remains retained while necessary claims or inputs are missing and group fulfillment remains pending. No categorical rerun of valid partial work is required merely to serialize a batch; the configured combined assessment remains necessary.

An independently configured singleton group can qualify when its own inputs and required comparison are ready, without waiting for unrelated groups. This includes the default separate intent-sufficient ordinary and challenge groups described in LE-173. The challenge uses the accepted axis-level alignment claim and comparison, not a fabricated whole-gate pass.

### LE-178: Explicit, durable required-author-count amendment
- Status: live
- Coverage: e2e/journey

The owner may amend these counts directly or explicitly delegate count-changing authority to a driver. Standing delegation covering the amendment is sufficient; no new owner interaction is required for every change within that authority. General autonomy alone is not silently interpreted as permission to change review rigor. Durably record the applying actor and the authority used, referring to existing owner directions or an explicit good-faith declaration as appropriate; do not require private-session verification or a new permission registry. The same rule covers increases and decreases.

A supported active run can explicitly amend the required number of authors for its current or future configured review, acceptance-criterion and whole-goal judgment obligations without starting a replacement run. The supported path guides the driver, validates known targets and positive counts, records the before/after counts and attributed amendment, and exposes the effective counts in inspection and subsequent checked fulfillment. Original initial input and earlier policy/evidence history remain truthful rather than being overwritten. This is a normal authorized amendment, not an override or bypass.

Only the declared author-count change takes effect. Configured group membership, judging meaning, stage relationships, reviewer eligibility, independence, blindness and self-review permission remain binding. Coupled individual/aggregate author requirements must remain consistent; an amendment cannot silently make those relationships impossible or waive them. Criterion and whole-goal author floors can be explicitly targeted as well; a review-count amendment does not silently change them.

Earlier judgments, actual instructions/returns, findings and completed decisions keep their original identities and history. Lowering a floor does not discard a standing failure or unresolved finding, manufacture a pass, relabel old review work, or erase a previously counted cold reader's material alternative. Existing genuine judgments remain usable where their scope and meaning remain applicable. Raising a count needs additional eligible judgments when existing coverage is insufficient. Completed decisions are not silently rewritten under the new counts.

The driver continues ordinary checked progression under the effective counts. This capability does not retrofit amendments into an unsupported older runtime or migrate its guidance or obligations.

### LE-179: Honest final residual reporting through existing surfaces
- Status: live
- Coverage: e2e/journey

Software-change final reporting and its configured criterion/whole-goal and validation review expose the material limitations, accepted residual risks, unresolved findings, unsupported operations and relevant proof uncertainty that remain at the delivered state. Distinguish an intentionally accepted scope/risk boundary from an undelivered stated outcome, missing proof and a separately authorized exception. Identify whose actual decision accepted a residual and retain the relevant original evidence/disposition; do not attribute a driver decision to the owner or rewrite history to imply a clean pass.

A final reader can understand what was delivered, what remains, and whether completion is ordinary or exceptional without reconstructing the previous private conversation. Existing report, review and finding/evidence paths carry this information; no new workflow phase, duplicate residual ledger or automatic whole-run classifier is required. Concise references may preserve detail without repeatedly copying every historical comment. An accepted risk never waives a stated outcome, and an unresolved material defect is not renamed residual to obtain normal approval.

The final report and actual whole-goal judgment agree with retained facts. This is honest reporting and assessment, not a promise to discover every hypothetical future problem or a new backend invocation.

### LE-180: Small profile customization without owning a wholesale fork
- Status: live
- Coverage: e2e/journey

The guaranteed small-customization categories are common supported policy knobs: required author counts for reviews, criteria and the whole goal; explicit eligible self-review permissions; review groups/axis membership and individual/aggregate stage selection; and advice and Bookends switches. These can be expressed through the base-derived preparation path without owning the complete base file. Raw artifact-schema or prompt replacement may still use a complete validated custom profile; this criterion does not require a general editor for every possible field. These are pre-start policy choices, not permission to change semantic group/stage/eligibility policy in an active run.

Before starting a supported provider run, an operator can select a shipped base profile and express a small set of intended changes through supported preparation, without hand-maintaining the entire copied default profile. The effective profile is validated and inspectable with its actual stages, groups, author counts, eligibility, advice and other relevant supported obligations. The operator can distinguish their overrides from the base defaults and see the base identity and any drift or divergence; a custom name cannot silently select different semantics.

For future preparation using a newer shipped base, the unoverridden remainder comes from that selected base rather than a stale wholesale copy. Changed defaults, conflicting or no-longer-applicable overrides remain visible rather than being silently accepted. Resolve and confirm the actual effective profile before launch. A started run does not silently track subsequent shipped changes: its original resolved input remains recorded, with only separately supported explicit amendments such as LE-178 changing active counts.

Preserve required independence and self-review rules except choices expressly made through supported pre-start policy. Use the smallest adequate recipe/options/overlay path, not an inheritance framework, preference store, editor, automatic active-run migration or execution binding system.

### LE-181: One workflow can deliver across multiple repositories
- Status: live
- Coverage: e2e/journey

A driver can complete a supported software-change run whose intended work spans more than one Git repository, using the ordinary artifacts, externally performed work, evidence and checked progression rather than starting a separate run merely because work crosses a repository boundary. Applicable engine/shared and other-provider evidence/reference paths must not assume every relevant source lives beneath one evaluation cwd; retain each provider's existing target and phase meaning rather than inventing a new multi-target workflow.

The driver declares the relevant repository/source scopes. Artifact coverage, source references, command proof and repository/checkpoint identities unambiguously identify the appropriate repository and revision/state. The same relative filename or local requirement ID in two repositories does not make them interchangeable. Required affected work and proof in a second repository cannot be omitted or passed by reusing an unrelated primary-repository success. Applicable Bookends and other authority obligations are assessed in their correct repository scope; do not silently waive them or merge distinct authorities into one unqualified ID list.

Existing source/evidence may be retained where genuinely applicable under LE-171, with current affected checks and honest identity/history. Keep declarations and mechanical checks proportional; use the existing durable references/captures and driver judgment rather than a mandatory external-work registry, repository manager or automatic semantic-impact classifier. The driver owns checkout/worktree/clone/merge/commit lifecycle and its authorization; the engine/provider gains no such ownership.

### LE-182: Clear ordinary findings before challenging the clean conclusion
- Status: live
- Coverage: e2e/journey

For a review group, ordinary review finds issues first. The driver triages them, makes required corrections and obtains affected ordinary confirmation. Repeat that ordinary sequence until no accepted material finding remains unresolved. The corresponding challenge then tries to falsify the cleared conclusion by finding an actual remaining material defect; it does not adjudicate open ordinary failures. It may examine prior findings, corrections and dismissals as evidence against that clean conclusion. Disputed ordinary findings use separate reconsideration during triage. Challenge findings return through the normal triage, correction and affected ordinary-confirmation path before the revised clean conclusion is challenged. Confirmation may focus on affected obligations and regressions; unchanged judging meaning may use the explicit applicability in LE-171 rather than restarting a broad search each time. This ordering governs challenge fulfillment, not inert early preparation in LE-174 or qualification independent of unrelated groups in LE-177.

### LE-183: Externally executed command results fulfill normal proof obligations
- Status: live
- Coverage: e2e/journey

Drivers and harnesses run proof/test commands and use supported recording to retain their genuine results, actual command/input settings, output or accessible output references, and relevant repository/source identity and applicability. Known metadata can be completed from the actual request/context, but missing outcomes or substantive evidence are not invented. Results are honestly attributed as externally produced, not engine-executed or engine-captured. Existing genuine historical engine captures remain original history and may be used where applicable; they are not relabeled as new external execution.

Configured normal command-proof obligations have a supported external-result fulfillment path. Sufficient genuine results are not rerun merely to obtain an engine-owned capture, and the new product has no optional proof-command/matrix runner. Mechanical source/target/command/currentness checks and actual semantic proof relevance still govern admission and completion. Exit zero, valid metadata or an all-green summary alone does not establish delivery. Missing, wrong-repository, stale, failed or insufficient results remain pending or fail under the existing rules; correction/reuse cannot fabricate them.

Preserve actual output and original attribution, and distinguish recording acceptance from independent judgment that command assertions prove the intended result. Deterministic provider calls and metadata inspection are not primary work execution. No private-call verification, arbitrary evidence quota or duplicate command-provenance registry is required.
