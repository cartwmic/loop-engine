# Loop Engine v2 — Product Requirements Document

**Status:** Living
**Target:** v0.1
**Compatibility:** Clean-slate successor; no v1 compatibility requirement
**Amended:** 2026-08-10

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
- automatic retries;
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
```

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

The semantic v0.1 surface contains at most seven primary operations. Exact CLI spelling remains evolvable during v0.x.

| Operation | Purpose |
|---|---|
| `start` | Create a run from a provider and initial input |
| `list` | Discover runs |
| `show` | Obtain the current working context |
| `append` | Add a context record |
| `event` | Request workflow progress |
| `history` | Inspect semantic run history |
| `terminate` | Explicitly close an active run |

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
```

The projection is the **chronologically latest durable evaluation** for each exact checked transition. An `allow` supersedes any earlier `deny`, and a later `deny` likewise supersedes an earlier `allow`. When the latest result is `deny`, its actionable feedback is exposed.

The projection is scoped to exact checked transitions, not merely currently requestable events. This preserves useful review feedback across revision edges without turning evaluation results into context records.

`show` does not invoke the provider.

A fresh actor **with access to any externally referenced work** must normally be able to resume from `show` without the previous session or raw history. Any workflow-specific external location or identity needed for handoff must therefore be carried in initial input, context records, or state instructions rather than ambient prior-session state.

### 6.3 Append

Conceptually:

```text
append <run> <kind> <JSON data>
```

Appending:

- creates one immutable context record;
- creates one semantic history entry;
- preserves stable append ordering;
- does not change workflow state;
- does not invoke the provider.

Only active runs accept context records.

`append` against a terminal run is rejected and creates no semantic history.

### 6.4 Event

Conceptually:

```text
event <run> <event>
```

For a syntactically valid request against an existing run, absence of a matching transition from the current state is `rejected`, regardless of whether the event ID appears elsewhere in the workflow.

For a matching transition, the engine:

1. loads the authoritative active run;
2. resolves the exact transition from the stored workflow;
3. for a check-free transition, atomically commits the target state and one aggregate history entry without invoking the provider;
4. for a checked transition, constructs the evaluation request and asks the provider to evaluate that exact transition;
5. after the provider returns `allow` or `deny`, verifies that the run is still active and remains in the source state against which evaluation began;
6. if that state/lifecycle check fails, treats the evaluation as stale, returns an error, and records no semantic history or evaluation lineage from the stale result;
7. on a non-stale `allow`, atomically commits the target state and one aggregate history entry;
8. on a non-stale `deny`, preserves state and atomically records one aggregate denied-transition history entry containing the feedback;
9. on `unsupported` or operational failure, preserves state and returns an error without adding semantic run history.

v0.1 does **not** invalidate an in-flight evaluation merely because context records or evaluation history changed concurrently. The supported usage model assumes one logical mutating actor.

### 6.5 History

`history` returns the ordered semantic history of the run.

It exists for understanding progression, prior review decisions, context additions, and termination.

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

`terminate` closes an active run without provider evaluation and records one termination history entry.

Final and terminated runs cannot be terminated again.

A terminal `terminate` request is rejected and creates no semantic history.

A terminated run cannot reopen in v0.1.

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
```

The engine validates the definition before creating a run according to the structural-validity requirements in Section 4.1.

In v0.1, workflow topology is **run-input-independent**: an individual run's initial input or context does not change the workflow definition produced for its provider association. Run-specific information influences the work and provider evaluation, not the workflow topology returned by `describe`.

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

Required topology:

```text
explore
  └─ intent-ready [checked] → design

design
  └─ design-ready [checked] → design-review

design-review
  ├─ approved [checked] → plan
  ├─ revise [check-free] → design
  └─ revise-intent [check-free] → explore

plan
  └─ plan-ready [checked] → plan-review

plan-review
  ├─ approved [checked] → implement
  ├─ revise [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

implement
  └─ implementation-ready [checked] → implementation-review

implementation-review
  ├─ approved [checked] → validation
  ├─ revise [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

validation
  ├─ passed [checked] → end
  ├─ revise [check-free] → implement
  ├─ revise-plan [check-free] → plan
  ├─ revise-design [check-free] → design
  └─ revise-intent [check-free] → explore

end [final]
```

The provider may inspect repository state, documents, tests, reviews, or other software-specific information. Core understands none of those concepts.

Review-state routing is explicit in this reference graph: external review operators select the phase owning an accepted material defect through `revise-intent`, `revise-design`, or `revise-plan`; validation-report-local defects stay in validation: edit and recheck `validation-report.json`, then retry checked `passed`; from validation, nearest `revise` is only for implementation-owned defects. Candidate reviewer output is triaged before append or artifact mutation, and disputed candidates use focused external reconsideration. A late finding requires current evidence, violated in-scope obligation, concrete consequence, validation gap, and provenance as newly exposed, fix-introduced, or previously overlooked; prior visibility or reviewer overlook does not waive known materiality. Comprehensive-first review still bars drip-feeding, and unrelated reopening must meet independent scope/materiality burden. A default three-round circuit breaker changes review method only and never waives a known defect. These are provider/operator conventions, not Loop Engine core policy or a review subsystem.

### 10.1 Run-Configured Semantic Review Policies

The software-change provider's shipped review contract remains external and deterministic at its boundary. Binaries built from this candidate source expose identity through `--help`/`-h` and `--version`/`-V` before stdin; public v0.2.2 binaries predate those flags. Direct pushes to `main` reuse read-only production preflight at the pushed SHA. Calibration fixtures are supplied-material-only and use stable fictional path labels with shipped companions, so review does not resolve labels against a live checkout. Stale evidence is recovery context under `details.informational`; current unsatisfied obligations remain under blocking `details.diagnostics`.

At run creation, the software-change workflow may receive semantic review policies in immutable initial input. These policies configure what must be reviewed at the review/validation gates without changing workflow topology.

The initial input may contain the equivalent of:

```text
change request / objective
repository or workspace reference, when required
semantic review policies grouped by review gate
optional supporting context
```

The review gates are:

```text
intent (explore → design)
design-review
plan-review
implementation-review
validation
```

A policy is a durable semantic requirement, conceptually identified by a stable workflow-specific ID and human-readable description. The exact input schema belongs to the software-change provider rather than Loop Engine core. Different runs may therefore use materially different review policies while retaining the same stored workflow topology. Because the policies are part of immutable initial input, the configured policy set for an existing run does not silently change during that run.

Configured policies are exposed to callers through the ordinary `show` projection because `show` already returns immutable initial input. They are **not formatted prompts** and the provider interface does not gain a prompt-generation operation. A human, agent harness, or other caller may render a policy into whatever review instructions or prompts are appropriate for that reviewer.

Semantic review work is performed externally. For each configured policy axis, a human, agent, script, or other system may append a workflow-specific context record containing the review result and actionable findings. The exact review-evidence schema is a software-change-provider convention; Loop Engine core treats it as opaque context and assigns it no truth, provenance, or approval authority.

For the reference software-change provider, checked progression at a review/validation gate validates whether the durable run data contains acceptable review evidence for the policies configured for that gate. It may deny because required evidence is missing or reports failure, and its denial feedback should identify the unsatisfied policy obligations. The reference provider does not itself perform the underlying semantic review and does not generate reviewer prompts.

This is a reference-workflow convention, not a new engine-level semantic-policy, validator, review-result, or prompt abstraction.

The workflow must support:

- a minimal initial idea;
- substantial initial context representing already-discussed intent/design/planning;
- semantic review policies configured per review/validation gate at run creation;
- different policy configurations across runs without changing workflow topology or core behavior;
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
```

Conceptually:

```text
context = show(run)

while context.lifecycle == active:
    perform current state's instructions
    append durable context when useful
    request an available event

    if rejected:
        use returned feedback and continue work

    context = show(run)
```

A harness may retain additional private conversational context, but correct workflow continuation must not depend on it.

No workflow-specific harness extension is required to enforce workflow progression.

## 14. Acceptance Criteria

v0.1 is complete when the following are demonstrated end to end.

### 14.1 Workflow Authority

- A caller cannot directly set current state.
- A structurally uninterpretable workflow definition cannot create a run, while structurally valid unusual topology such as cycles, unreachable states, non-final sinks, or absence of a final state is permitted.
- Workflow topology does not vary per run based on initial input or accumulated context.
- A syntactically valid but unavailable event is rejected without changing state or semantic history.
- A check-free transition commits from stored graph semantics without provider evaluation.
- A checked transition cannot advance without provider `allow`.
- Provider output cannot select a different target state.
- Rejected and errored requests preserve current state.
- Accepted transitions and their required history commit atomically.
- Current state survives process restart.
- Active runs retain their stored topology and instructions after the provider's current `describe` output changes.
- A provider implementation unable to evaluate a stored workflow/action fails explicitly without advancing the run.
- Final states cannot declare outgoing transitions.
- An initially-final run is created final.
- Terminal runs expose no requestable events and reject `append`, `event`, and `terminate` without semantic history.

### 14.2 Durable Context and Handoff

- Initial input is immutable after creation and survives restart.
- Context records survive restart in stable append order.
- Context records have no engine-level truth, provenance, approval, or supersession semantics.
- Every checked evaluation receives all accumulated context records in durable append order.
- `show` gives a fresh actor enough information to resume without the previous conversation or raw history, assuming access to externally referenced work.
- Workflow-specific external work identity required for handoff is durably represented through opaque workflow data or instructions rather than ambient session state.
- Appended steering is visible to subsequent actors and evaluations.
- `show` preserves review feedback across revision edges by exposing the chronologically latest durable evaluation per checked-transition lineage.
- Later durable evaluations supersede earlier ones in either direction: `allow` can supersede `deny`, and `deny` can supersede `allow`.

### 14.3 Run History and Evaluation Lineage

- Run history contains only the semantic actions defined in Section 4.4.
- Reads, unavailable events, unsupported evaluations, and operational failures do not pollute semantic history.
- One transition request creates at most one aggregate transition history entry.
- History ordering remains stable across restart.
- A committed `allow` and durably recorded `deny` survive process and actor changes.
- Evaluation lineage is scoped to the exact checked transition, not a provider policy key.
- Later evaluations receive prior durable `allow`/`deny` lineage in stable order.
- `unsupported`, provider failures, stale results, and uncommitted evaluations do not enter evaluation lineage.
- Providers may use or ignore prior evaluations without different engine semantics.
- `allow` requires no durable feedback payload; `deny` carries durable actionable feedback.
- Raw run history is not implicitly supplied as evaluation context.

### 14.4 Concurrency

- Overlapping event attempts competing against the same pre-mutation run state cannot both produce conflicting commits.
- An in-flight checked evaluation made stale by a state or lifecycle change produces no transition, semantic history, or evaluation lineage regardless of whether the provider returned `allow` or `deny`.
- Concurrent context appends alone are explicitly outside v0.1 evaluation-staleness guarantees.

### 14.5 Software-Change Workflow

- A minimal idea can be driven to completion.
- Substantial already-known intent/design/planning context can use the same workflow.
- A run can configure semantic review policies for the review/validation gates in immutable initial input.
- The configured policies are available to a fresh actor through `show` without a provider call or separate policy-discovery operation.
- The same workflow topology and software-change provider mechanism can execute runs with materially different review-policy configurations.
- Human or agent reviewers can perform each configured semantic review externally and append the resulting pass/fail findings as ordinary durable context using the software-change provider's workflow-specific evidence convention.
- The reference software-change provider does not itself perform the semantic review or generate formatted review prompts; it validates whether the configured review obligations have acceptable durable evidence before allowing progression from a review/validation gate.
- Missing or failed required review evidence denies the corresponding checked approval with actionable feedback identifying the unsatisfied policy obligation.
- Acceptable evidence for all configured policies at a gate allows that policy portion of the gate to pass without introducing engine-level policy or review-result semantics.
- At least one review denial and revision cycle is demonstrated.
- After a review denial and check-free revision edge, a fresh actor can see the configured policies, durable review evidence, and actionable review feedback through `show` without reading `history`.
- After a later successful evaluation, the previous denial is no longer projected as the latest result for that transition.
- User steering can affect later work and evaluation.
- Prior review evidence and evaluation lineage can inform later review/check cycles.
- A run can move between distinct actor sessions or harnesses.
- A check-free revision edge remains usable when provider evaluation is unavailable.

### 14.6 Policy-Document Workflow

- Draft and audit modes both work.
- Deterministic policy failure blocks progression with actionable findings.
- Semantic policy failure blocks progression with actionable findings.
- Successive semantic reviews may use previous findings.
- Successive external semantic reviews may use previous findings; provider may use prior lineage only to inform validation diagnostics or evidence aggregation, or ignore it when validating current evidence.
- The actor can revise and request evaluation repeatedly until policies pass.
- After deterministic review passes, a later document revision that violates a deterministic policy cannot finalize until deterministic conformance is re-established according to the external state observed by provider evaluation.
- The workflow does not rely on Loop Engine atomically locking, versioning, or committing the external document together with workflow state.
- README-like and `AGENTS.md`-like policy sets require no core changes.

### 14.7 Research Workflow

- An operator can start a research run through Loop Engine using `start` / `show` / `append` / `event`.
- Topology covers scope, gather, adversarial verify, and synthesize.
- Checked transitions refuse until artifacts satisfy declared structure and independent evidence satisfies declared review obligations at verify and synthesize.
- Local blackbox tests exercise at least one checked denial and a successful completion.
- CI preflight builds the research binary and runs a source journey; archive-smoke runs a packaged journey after materializing embedded data.
- cargo-dist plan and release-gate assertions include the research binary.
- The provider does not fetch, invoke models, or judge semantic truth.

### 14.8 Operational Simplicity

- Local operation requires no daemon or external infrastructure beyond Loop Engine's local durable state and configured provider integration.
- The primary caller surface remains seven or fewer operations.
- The semantic provider interface remains `describe` + `evaluate`.
- Provider correctness does not depend on retained in-memory state from earlier invocations.

## 15. Complexity Guardrails

v0.1 deliberately targets:

```text
provider semantic operations: 2
primary caller operations:    ≤ 7
active states per run:        1
background services:          0
automatic retries:            0
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
checkpoints
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
