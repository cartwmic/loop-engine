# Reference Software-Change Workflow

**Status:** Required acceptance example and motivating illustration. The engine must support this workflow end to end through its production CLI, but this workflow does not prescribe the shape or subject matter of other workflows.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)

## Purpose

This example grounds `loop-engine` in a realistic software-engineering workflow with substantial provider-defined validation and guidance.

It demonstrates why the engine needs:

- code-only workflow providers;
- a normalized stored graph;
- provider-defined gates;
- explicit revision cycles;
- optional immutable run inputs;
- append-only evidence;
- static and dynamic guidance;
- durable authoritative state;
- an immutable explanatory journal;
- stable active-run topology across provider changes.

Future implementation must prove this workflow through black-box CLI E2E coverage. Exact command spelling, provider language, artifact filenames, and presentation formatting are not prescribed by this document.

## Non-prescription

This example uses software phases, files, templates, reviews, and an optional workspace because they motivate the product. They are provider concepts, not core concepts.

The engine must also permit workflows with:

- no files or artifacts;
- no workspace or repository;
- no run inputs;
- no evidence beyond transition outcomes;
- no review phases;
- different state/event vocabulary;
- external services or physical processes instead of software work.

Core understands states, events, transitions, gate requirements, gate verdicts, run inputs, evidence, current state, and journal facts. The reference provider interprets intent documents, designs, plans, reviews, implementations, validation reports, templates, and workspaces.

## Provider boundary

One immutable machine-local reference-provider registration ID is logical workflow identity and cohesive authoring source for:

- graph declaration;
- state instructions;
- optional run-input declarations and separate value validation;
- gate identifiers and implementations;
- static guidance;
- stored declaration of optional live-guidance support;
- evidence conventions.

At run creation, provider emits complete normalized graph. Engine validates and stores graph snapshot. Provider code never sets current state directly.

Reference provider may use shell scripts, a bespoke binary, or another executable implementation internally. Engine interacts only through provider protocol.

## Example run inputs

Reference provider may declare provider-free inspectable, non-secret immutable inputs such as:

- a change identifier;
- a stable work-root or repository locator;
- an artifact root;
- template or policy locations.

These names and meanings belong to reference provider. Different projects may use ordinary folders, worktrees, both, or neither. Engine does not create or interpret workspace topology. If external location moves, provider may define append-only remap evidence convention; otherwise caller restores location or creates new run.

Zero-input workflows remain valid.

## Required graph

```text
explore
  └─ intent-ready ─────────────────────→ design

design
  └─ design-ready ─────────────────────→ design-review

design-review
  ├─ approved ─────────────────────────→ plan
  └─ changes-requested ────────────────→ design

plan
  └─ plan-ready ───────────────────────→ plan-review

plan-review
  ├─ approved ─────────────────────────→ implement
  └─ changes-requested ────────────────→ plan

implement
  └─ implementation-ready ─────────────→ implementation-review

implementation-review
  ├─ approved ─────────────────────────→ validation
  └─ changes-requested ────────────────→ implement

validation
  ├─ passed ───────────────────────────→ end
  └─ failed ───────────────────────────→ implement

end [final]
```

Event names are part of reference provider's graph, not mandatory global vocabulary.

## State intent

### `explore`

Goal: determine intended result for this workflow run in current software-engineering context.

Expected provider-defined output: intent artifact capturing desired result, constraints, and success conditions.

`intent-ready` gate validates that expected intent exists and follows provider's required shape.

### `design`

Goal: produce technical design satisfying accepted intent.

Expected output: design artifact linked to relevant intent evidence.

`design-ready` gate validates existence, structure, and relationship to accepted intent.

### `design-review`

Goal: review design and persist verdict with findings.

Expected output: design-review evidence tied to exact design revision.

- `approved` gate validates review exists, addresses expected criteria, refers to current design, and carries approving verdict.
- `changes-requested` gate validates review exists, refers to current design, and carries revision verdict.

Revision returns to `design`; it is not modeled as exceptional retry.

### `plan`

Goal: produce actionable implementation plan based on accepted intent and approved design.

Expected output: plan artifact.

`plan-ready` gate validates plan existence and provider-defined shape.

### `plan-review`

Goal: review plan for feasibility, completeness, and alignment.

Expected output: plan-review evidence tied to exact plan revision.

- `approved` advances to implementation.
- `changes-requested` returns to `plan`.

Both paths require provider validation of matching review verdict.

### `implement`

Goal: carry out plan externally and persist provider-required completion evidence.

Engine does not edit code, invoke coding agents, or otherwise perform implementation work.

`implementation-ready` gate may validate implementation evidence, repository state, required reports, and project quality commands.

### `implementation-review`

Goal: review implementation against accepted intent, design, and plan.

Expected output: implementation-review evidence tied to implementation revision.

- `approved` advances to validation.
- `changes-requested` returns to `implement`.

### `validation`

Goal: validate completed result through provider-defined checks.

Expected output: validation evidence or report.

- `passed` advances to final state.
- `failed` returns to implementation with durable failure evidence.

### `end`

Final state whose provider-defined domain meaning is successful workflow completion. Engine lifecycle reports neutral `final`. No primary work remains actionable.

## Gate semantics

Engine knows gate IDs from stored graph but does not understand software-specific validation.

Reference provider gates may validate:

- expected path exists;
- artifact follows template or schema;
- required sections and relationships are present;
- review refers to exact subject revision;
- review verdict matches requested event;
- repository or external system satisfies expected condition;
- project test/validation command succeeds.

All gates required by event are evaluated in one provider invocation against bounded snapshot containing current state, selected event/transition, immutable inputs, inline evidence, and caller-selected prior-evidence records/references. Provider returns exactly one semantic result: complete pass/fail verdict set with optional valid evidence, explicit stored-graph incompatibility, or evaluation error. Gate fail and explicit incompatibility reject transition. Evaluation error, missing executable, timeout, unsupported protocol major, missing/malformed result, or malformed provider evidence produces operation error. Current state remains unchanged.

Provider verdict authorizes only transition selected from stored graph. Provider cannot compute or directly apply another target state.

## Guidance

Stored graph includes static state title, summary, and actor-neutral instructions sufficient for cold provider-free resumption, or explicit declaration that no additional guidance is needed.

Stored projection declares whether reference provider supports live guidance. When supported, provider may generate it from current run context, external files, tools, or services. Caller explicitly requests live guidance through production CLI. Engine does not treat it as transition authority. Ordinary static inspection remains provider-free.

## Evidence

Accepted and rejected attempts may append evidence such as:

- logical evidence kind;
- locator;
- content digest;
- media type;
- provider or validator identity;
- bounded diagnostics;
- opaque provider metadata.

If provider reuses locator for revised artifact, new evidence gets stable run-scoped ID and is appended. Prior evidence is not overwritten. Provider-free inventory exposes IDs and metadata for later selection. Engine stores bounded evidence record as submitted and does not automatically dereference or copy external locator content.

Actor notes and opaque identity metadata may accompany evidence but never grant transition authority. Independently verifiable role or credential evidence may satisfy provider-owned gate policy.

## Journal expectations

Reference workflow journal must explain:

- run creation and provider identity;
- stored graph identity;
- requested event;
- state and workflow-state/lifecycle version before request;
- provider invocation identity/digest when available;
- each required gate verdict;
- bounded diagnostics and evidence references;
- completed, domain-rejected, or operation-error outcome with detailed reason;
- state and workflow-state/lifecycle version after completed transition;
- provider drift;
- optional actor metadata and note.

Journal is not required to reproduce provider execution, retain external evidence content, rebuild current state, or prove provider process never started before abrupt engine-process death.

## Provider drift

Active run keeps creation-time graph even when reference provider changes.

Updated provider implementation/policy is used and gate-attempt digest is logged. It must honor active run's stored declarations/guidance contract or explicitly report requested capability incompatible.

Updated provider may emit different graph for new runs. Active run does not adopt it automatically.

## Required black-box acceptance behavior

Future production CLI E2E suite must prove all following using real executable reference provider and real persistence:

1. **Creation and safe inspection:** run is created from provider-emitted graph; later safe inspection uses stored graph without requiring provider graph discovery.
2. **Happy path:** caller can advance through every forward state to `end` when provider requirements pass.
3. **Missing output rejection:** missing expected output rejects transition, preserves current state, and appends explanatory journal information.
4. **Invalid output rejection:** output that exists but violates provider shape/template also rejects without state mutation.
5. **Design revision cycle:** `changes-requested` returns design review to `design`; corrected design and later approval can advance.
6. **Plan revision cycle:** `changes-requested` returns plan review to `plan`; corrected plan and later approval can advance.
7. **Implementation revision cycle:** `changes-requested` returns implementation review to `implement`; corrected work can return for review.
8. **Validation revision cycle:** failed validation returns to `implement`; later successful validation can reach `end`.
9. **Verdict consistency:** review event and provider-validated review verdict must agree; mismatch rejects.
10. **Append-only evidence:** revised artifact at same logical path records new evidence without rewriting prior evidence.
11. **Restart and handoff:** separate CLI process can inspect and continue run after every state-changing operation.
12. **Provider drift:** provider implementation/policy change is logged by gate-attempt digest and allowed while active projection remains unchanged; incompatible requested capability rejects explicitly.
13. **Provider incompatibility:** explicit compatibility check completes with incompatible finding for missing required stored gate; provider-dependent event returns domain rejection without advancing while run remains inspectable, annotatable, and terminable.
14. **Guidance and cold handoff:** provider-free show supplies inspectable inputs, sufficient static current-work guidance, stored live-guidance capability, and empty-default caller-owned evidence selection; history/evidence inventory exposes stable IDs/associations; live guidance may recommend IDs without authorizing transition or auto-selection.
15. **Actor neutrality:** same run request, evidence, and provider verdict produce same engine decision regardless of actor metadata.
16. **Journal/state consistency:** every completed transition atomically updates authoritative state and appends matching journal record; rejection never reports target as current.
17. **Interaction contract:** active run is discoverable from another working directory; provider-free show reports requestable events; optional label changes only while active; terminal run never reopens but accepts append-only annotation.
18. **Attempt evidence:** after run lookup, inline/selected/provider evidence is retained and reported atomically for completed, rejected—including unknown event/lifecycle denial—and operation-error attempts whenever persistence remains available; abrupt process-death limits are explicit.
19. **Provider resolution:** active run uses stable machine-local registration ID to resolve current executable and explicit working directory independent of caller CWD, while retaining stored graph and journaling observed invocation facts.
20. **Automation envelope:** each dispatched structured result emits one stdout envelope and distinct completed/rejected/error exit behavior.
21. **Operational visibility:** every reference operation produces a correlated per-invocation trace with bounded request/outcome payloads; provider and persistence paths expose their boundary events, and trace-initialization failure performs no operation/provider/mutation work.

Tests may combine behaviors into fewer scenarios, but every behavior must have explicit runtime-observed coverage. Generic fixtures, not this reference graph, own self-loop and other graph-feature facets absent from reference workflow.

## Motivation without prescription

This example is intentionally rich enough to pressure architecture:

- substantial custom provider code avoids pushing logic into configuration language;
- graph snapshot keeps engine authority inspectable;
- explicit cycles model real revision work;
- optional inputs support folders, worktrees, and other environments without core workspace semantics;
- append-only evidence records changing artifacts without mutable context bag;
- provider drift favors iteration while stable graph protects active-run meaning;
- state plus journal supplies practical auditability without replay machinery.

Future simplification must preserve required acceptance behavior. Future generalization must not turn example's software-specific vocabulary into core policy.
