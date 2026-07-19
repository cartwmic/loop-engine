# Loop Engine Core Tenets

**Status:** Living foundation. Tenets guide trade-offs; [invariants](invariants.md) state behavior that must hold.

Related documents:

- [Product intent](intent.md)
- [System invariants](invariants.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)

## 1. Actor neutrality

Human, autonomous agent, script, and external system are equivalent performers of work. Actor type must not change workflow semantics.

Identity may be recorded as optional opaque audit metadata. It is not a source of transition authority.

## 2. Harness neutrality

Core behavior must not depend on Claude Code, Codex, Pi, or any other harness. Harness and model preferences may exist in caller tooling outside engine; MVP defines no engine semantics for them.

## 3. Inversion of control

The engine coordinates work; it does not perform or initiate primary work. A caller asks what is actionable, performs that work elsewhere, then submits an event and evidence.

Provider gate evaluation is different: the engine invokes providers because it must own transition enforcement. Arbitrary provider code may have side effects, but the provider protocol must not grant it authority to set run state directly.

## 4. Workflow policy belongs to providers

Executable workflow providers are the sole authoring source for workflow-specific graph declarations, gates, guidance, evidence conventions, and metadata.

Core supplies generic mechanisms for states, events, transitions, gate verdicts, evidence, state persistence, and journaling. It must not contain hidden policy such as requiring a proposal, review, commit, artifact, workspace, or particular tool.

## 5. The engine owns the graph projection

Provider emits complete normalized graph, static guidance, and live-guidance capability without candidate inputs; separate operation validates input values without changing projection. Engine validates and stores full projection for run. Provider code may describe and evaluate workflow policy, but only the engine resolves events against the stored graph and changes current state.

## 6. Evidence outranks completion claims

A caller saying “done” is a request to evaluate a transition, not proof that transition requirements are satisfied. Provider-defined gates determine whether progress is allowed.

## 7. Every meaningful change is explainable

Auditability is part of correctness. Anyone inspecting a run must be able to determine what engine durably observed: what was requested, in what order, which provider results and gates were involved, why transition completed or rejected, and what bounded evidence record was stored.

Explainability does not imply deterministic replay, historical re-execution, retained external evidence content, or complete observation across abrupt process death.

## 8. Current state is authoritative

The engine persists authoritative run state directly. The immutable ordered journal explains changes but is not folded to derive current state.

Authoritative run records and journal must never silently disagree about durable mutation.

## 9. Cycles are normal

Revision, retry, review, and refinement paths are ordinary workflow edges. The model must not treat cycles as malformed DAGs or hide them behind exceptional retry behavior.

## 10. Durable handoff

A process, session, human, or agent may disappear between steps. Another actor must be able to inspect persisted state and continue without relying on private conversational context or process memory.

## 11. Human and machine interfaces share semantics

Human-friendly output and structured output are two representations of the same operations and state. Machine use must not require a privileged or semantically different API.

## 12. Provider code is explicit executable content

Provider validation, run creation, gate evaluation, compatibility checking, and live guidance may execute arbitrary workflow code. Safe inspection of an existing run uses stored data without provider execution. Live provider guidance is explicitly requested rather than hidden inside ordinary inspection.

Configuring explicit machine-local provider registration authorizes its executable with caller permissions. The engine must expose execution clearly, fail closed on provider errors, bound captured output, and avoid claiming arbitrary provider code is sandboxed. MVP has no separate trust database, provider scanning, or approval ceremony.

## 13. Active graph is stable; provider implementation may evolve

Each run keeps its creation-time graph snapshot. Provider implementation changes are allowed and logged. New provider graphs affect new runs only. MVP does not migrate active graphs.

This trades executable and gate-policy reproducibility for iteration speed. Active-run topology, declarations, and static guidance stay fixed; gate policy remains live. Provider digest in journal makes drift visible, and provider must report incompatibility when current policy no longer honors stored contract.

## 14. Focused core over broad platform

Build for one local user operating one or several independent runs. Do not build multi-user coordination, scheduling, distributed workers, service orchestration, visual modeling, mutable workflow variables, or general expression language without demonstrated need. Preserve extension seams without paying speculative complexity up front.

Executable providers are the extension boundary for workflow-specific logic; they must not become a second state engine.

## 15. Reuse concepts without surrendering product boundaries

Statecharts, external-task patterns, activity journals, subprocess protocols, and existing workflow engines are valid prior art. Reuse libraries or standards when they preserve actor neutrality, external work, engine-owned state, gate authority, auditability, and practical local operation.

## 16. Rust is pragmatic, not ideological

New project code should prefer Rust when Rust fits. Providers and mature integrated components may use other languages when their value exceeds packaging, runtime, licensing, and operational costs.

## 17. Fail closed and explain why

Invalid provider graphs, unavailable or incompatible providers, gate failures, ambiguous transitions, persistence failures, and malformed evidence must produce explicit diagnostics. They must not silently advance a workflow or fall back to weaker behavior.

Public UX uses three outcome classes: completed operation, domain rejection, and operation error. Rejection means request was understood but denied; error means evaluation or commit could not complete. Detailed reason codes retain recovery information without expanding top-level vocabulary.

## 18. Clean-room design

Derive behavior from this project's intent and requirements. Do not inherit prior loop-specific designs or artifacts. Do not use OpenSpec-related skills, commands, or artifacts for this project.

## 19. Dependencies point inward

Core owns workflow meaning, operations, and capability contracts. Integrations implement provider invocation, persistence, configuration, and other external capabilities. CLI translates user interaction and wires concrete implementations. Outer technology must not leak into core types or policy.

Layer count is contingent. Dependency direction is the invariant.

## 20. Test through production behavior

Only tests exercising a production driver establish behavioral acceptance. An operation is complete when the real CLI invokes it against real persistence and executable provider fixtures and black-box tests verify its observable contract.

## 21. Prefer real integrations over mocks

Mock frameworks and mock-based behavioral tests are prohibited. Optional pure model/property tests need no mocks and may supplement, never replace, driver tests.

## 22. Completeness must be mechanically checked

Operation coverage must not depend on prose claims or manually asserted labels. The core operation catalog, operations exposed by drivers, and operations observed in passing end-to-end scenarios must agree mechanically.

## 23. Inputs establish context; evidence accumulates

Run inputs are optional provider-declared, provider-free inspectable, non-secret facts validated and frozen at creation; MVP graph does not vary by their values. Separate registration represents alternate topology. Evidence is optional evolving information with stable listable IDs, appended during execution. External-resource rebinding is provider convention or new-run recovery, not mutable input. Callers may append evidence independently or include it with an event request. Core does not require artifacts, workspaces, repositories, or files and does not provide a general mutable workflow-variable bag.

## 24. Run interaction stays small

A run is active, neutrally final, or explicitly terminated. Final state's provider-defined identity carries domain meaning; lifecycle does not label every final as success. Terminal run never reopens or accepts events but remains inspectable and annotatable. Stopping caller already suspends activity because engine performs no background work; MVP needs no pause status, revision-token ceremony, claims, leases, idempotency keys, automatic retries, or individual-run deletion.

Machine-local catalog shows active runs by default. Normal operation output emphasizes outcome, current state, whether state identifier changed, and next requestable events. Completed self-loop records transition while reporting unchanged state.

## 25. Provider compatibility is explicit and bounded

Immutable machine-local provider registration ID is logical workflow identity; mutable human handle is unique among enabled registrations and resolves to that ID. Active runs use current explicitly configured executable and working directory from stored registration ID while retaining creation-time graph. Engine computes canonical graph digest. Provider protocol maintains compatibility within major version; breaking changes require new major. Compatibility reports are non-latching and capability-scoped. They prove declared structural support, not unchanged gate policy. Safe inspection remains available when provider is missing or incompatible.

Engine provides schemas, examples, diagnostics, and boundary conformance checks rather than requiring official language SDK or claiming provider gate logic is correct.

## 26. Visibility is paramount for debugging

Engine work must not happen silently. Every CLI invocation creates an always-on structured operational trace before dispatch. Rich errors and trace records expose what engine was doing, which bounded payloads and external results it observed, which consequential decisions it made, and where failure occurred.

Instrumentation belongs at stable operation-dispatch, provider-execution, and persistence boundaries, with targeted events for consequential internal decisions. Do not require logging in every helper or add compiler/lint machinery that proves only presence of logging syntax.

## 27. Documentation converges at every publication checkpoint

Every accepted push must leave its destination tip coherent with behavior, architecture, contracts, testing policy, and development policy introduced by the aggregate remote-base-to-local-head change. Commits inside one unpublished push range may be incomplete and may repair one another; they are working history, not separate publication authorities. A later push cannot repair an incoherent checkpoint that should have been rejected.

The semantic judge evaluates the exact destination-tip-to-candidate-tip diff and resulting tree once. Deterministic documentation checks and semantic judgment are complementary. Judge remains replaceable development tooling and must not create engine runtime or harness dependency.
