# Loop Engine Intent

**Status:** Living foundation. This document records product intent, not detailed design.

Related documents:

- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Reference workflow](reference-workflow.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)

## Purpose

Build `loop-engine`: a CLI for creating, running, inspecting, and managing durable loop workflows supplied by executable workflow providers.

A loop workflow coordinates work performed outside the engine. The performer may be a human, an autonomous coding agent, a script, or another system. Engine exposes current work, accepts events and evidence, invokes provider-defined validation gates, persists authoritative run state, and records ordered journal explaining what happened.

MVP is local tool for one user operating one or several independent runs. It preserves data integrity across accidental process overlap but is not multi-user service or same-run collaboration system.

## Problem

Coding workflows often spread control across prompts, shell scripts, agent-specific extensions, mutable files, and human memory. This makes them difficult to resume, validate, transfer between actors, or audit after the fact.

Generic workflow engines can model parts of this problem, but commonly optimize for scheduled jobs, service calls, worker execution, DAG throughput, or centralized infrastructure. `loop-engine` focuses on externally performed work, provider-defined gates, cycles, local operation, durable handoff, and understandable history.

Declarative workflow files also create a tension when real workflows need substantial custom validation and dynamic guidance: graph declarations and executable policy become separate manually synchronized sources. `loop-engine` resolves this by making executable providers the sole workflow-authoring source while requiring each provider to expose a normalized graph projection that the engine validates and stores.

## Desired outcome

A workflow provider can define in one cohesive codebase:

- states and initial state;
- actor-neutral work instructions;
- events that may be submitted from each state;
- permitted transitions;
- named gates that must pass before a transition;
- optional immutable run-input declarations;
- static guidance and whether explicit live guidance is supported;
- append-only evidence and metadata conventions.

A caller can:

- discover active runs through machine-local catalog from any working directory;
- create durable run with stable ID and optional non-unique display label;
- inspect run's stored graph and current work without rerunning provider discovery;
- explicitly request provider-generated live guidance when supported;
- perform primary work outside the engine;
- append evidence and notes independently or submit them with an event;
- request named events and receive one of three clear result classes: completed, domain-rejected, or errored;
- see current state, whether it changed, and next requestable events after operation;
- stop and later continue from persisted state;
- inspect an immutable ordered journal of requests, evaluations, and outcomes;
- correlate every CLI invocation and rich error with always-on structured operational trace.

The [interaction storyboards](ux-storyboards.md) define end-to-end caller and provider-author experience. The required [reference software-change workflow](reference-workflow.md) grounds this outcome in an acceptance example while remaining non-prescriptive for other workflow domains.

## Product boundary

`loop-engine` owns:

- invoking a provider to obtain a complete workflow graph;
- validating and snapshotting that graph for each run;
- machine-local run catalog, lifecycle, and authoritative current state;
- deterministic transition resolution against the stored graph;
- provider gate invocation and verdict enforcement;
- evidence references;
- transactional state and journal persistence;
- human-readable and machine-readable CLI interaction;
- per-invocation rotating operational diagnostics across dispatch, provider, decision, and persistence boundaries.

`loop-engine` does not own:

- invoking or supervising coding agents;
- performing workflow-defined primary work;
- deciding whether work should be performed by a human or machine;
- implementing harness-specific session protocols;
- interpreting workflow subject matter;
- allowing provider code to set run state directly;
- guaranteeing arbitrary provider code is safe, deterministic, or side-effect free;
- guaranteeing historical execution can be reproduced.

## Workflow-provider model

Workflow authoring is code-only. A provider may be a shell script, Rust binary, Python program, Node program, or any executable implementing the provider protocol.

Immutable machine-local registration ID is logical workflow identity. Mutable human handle is unique among enabled registrations and resolves to ID. Registration uses explicit executable locator, arguments, and working directory. Active run stores ID; caller working directory, handle rename, and project defaults cannot rebind it. Same executable may back several registrations. No provider scanning, registry, installer, or automatic package discovery is required.

Provider is single authoring source for graph declarations, input validation, gate implementations, and guidance. Input-free description emits normalized graph/input declarations/static guidance/live-guidance capability; separate value-only operation validates candidate inputs without returning topology. When both creation calls run, they use same resolved registration and observed executable digest when available; detected executable change errors, while interpreted dependencies/environment remain outside digest guarantee. Engine validates full projection, computes canonical digest as graph-revision identity, and stores fixed projection.

Provider implementation and gate policy may change while run is active. Engine resolves current executable/working directory through stable registration, records actual locator/digest on gate invocation, and continues to apply stored projection. Topology/declarations/guidance remain fixed; gate policy is live and not historically certified. Changed provider must report incompatibility when it no longer honors stored contract. Compatibility reports are capability-scoped and non-latching.

MVP authoring support consists of a stable-major provider protocol, schemas, examples, actionable diagnostics, and engine-run conformance checks. An official language SDK is not required. Later SDKs, templates, generators, or wrappers must use the same provider protocol rather than create another workflow semantics.

Run inputs are optional, provider-declared, provider-validated, provider-free inspectable, non-secret, and immutable after creation. MVP graph projection is independent of input values; alternate topology uses separate registration. Evidence is optional, receives stable run-scoped IDs, remains provider-free listable, and accumulates by appending. Moved external resources require provider-defined evidence remap, location restoration, or new run; engine does not mutate inputs. Artifact, workspace, repository, and file concepts are not required by core; a provider may model them through its own inputs, evidence, and guidance.

## MVP direction

MVP targets full local lifecycle: register and validate providers, create and list runs, inspect current work, explicitly request optional live guidance, append evidence/notes, request events, enforce transitions, continue interrupted work, terminate runs, and inspect history.

Run lifecycle has three concepts: active, neutral final on entry to final workflow state, and explicit termination. Final state ID/provider metadata carries domain meaning. Final and terminated runs never reopen or accept events, expose no requestable events, but remain inspectable and annotatable. No pause/resume status is needed because engine performs no background work. MVP does not delete individual runs.

The accepted MVP engine remains narrow:

- one active workflow state;
- explicit event-driven transitions;
- named provider gates;
- deterministic transition acceptance or rejection once required gate verdicts are observed;
- cycles and self-loops;
- authoritative stored state;
- immutable ordered activity journal.

Hierarchical states, parallel regions, timers, child workflows, compensation, distributed workers, automatic primary work, mutable workflow variables, and embedded expression languages remain outside MVP unless later requirements justify a broader engine.

## Audit promise

Run journal explains durably observed run activity: what was requested, when it occurred, which provider identity/digest and result were observed, what gate verdicts were returned, what bounded evidence record or external reference was submitted, and why state did or did not change. Registration-wide compatibility reports and registration configuration changes are not copied into per-run journals. Engine does not automatically copy content behind external evidence locators and does not promise those resources remain available. Abrupt process death may prevent recording provider process that engine never observed complete.

Journal is not source of current state and need not support deterministic replay, historical re-execution, or full prior-state reconstruction. Current run state is authoritative. Every durable run mutation or attempt record and attached evidence must commit atomically. Rejected creation has no run and therefore no run journal.

## Operational visibility

Every CLI invocation initializes one current-user-only structured JSONL trace before operation dispatch. Failure to create secure trace prevents every operation. Trace records all bounded engine/provider payloads without redaction, provider streams, consequential decisions, and persistence outcomes; inherited process environment is excluded. Rich errors report request ID, failing phase, and trace location when available.

Instrumentation stays concentrated at operation dispatcher, provider adapter, and persistence adapter, with targeted internal decision events. Operational trace is rotating diagnostic material, not authoritative run state or journal. Rotation, storage failure, and abrupt death limit retention/completeness.

## Behavioral acceptance direction

The production CLI is the authoritative behavioral test driver. Every application operation must be reachable and regression-tested through at least one production driver; CLI is the only current driver. Lower-level tests cannot substitute for missing driver coverage.

Black-box end-to-end tests exercise the shipped binary with real persistence and real executable provider fixtures. Every catalog operation must also be observed in passing required E2E trace envelopes. Mock-based behavioral tests are excluded. Pure core property tests remain optional supplemental tools for combinatorial exploration, but they do not count toward operational completeness.

## Configuration direction

Machine-local provider registrations are authoritative and include explicit executable, arguments, working directory, and configurable timeout; configuring them authorizes execution with caller permissions. Global/project configuration may supply CLI defaults or registration references but cannot redefine registration selected by existing run. Machine-local catalog remains addressable independent of caller working directory. Provider registrations, stable workflow identities, run state, graph snapshots, transition evidence, metadata, and journal entries persist where required by semantics. Exact paths and configuration format remain undecided.

## Clean-room constraint

This project begins from the intent recorded in these documents. It must not import prior loop implementations, artifacts, or assumptions. OpenSpec-related skills, commands, and artifacts are outside this project's workflow.
