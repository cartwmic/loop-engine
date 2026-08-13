# Software-Change Provider PRD — Loop Engine v2

**Status:** **Frozen 2026-08-11** (v1.0) — three independent review rounds passed (dispositions: `.pi-subagents/artifacts/policy-comparison/prd-review-sol-disposition.md`). Acts as the authoritative intent for this change; further changes are explicit amendments.
**Owner:** cartwmic. **Date:** 2026-08-11.
**Evidence base:** `loop-engine-v2-software-change-policy-comparison.md`, `loop-engine-v2-software-change-playbook.md`, v1 provider rubrics (chezmoi `63771fb9e560…`), engine PRD (`docs/PRD.md`).

## 1. Problem

When I run a software change through Loop Engine today, I get false assurance: a run can traverse every gate and finalize a change nobody actually reviewed, while every transition is labeled "checked." A fresh actor resuming a run cannot tell what review obligations exist or whether they were met, because nothing inspectable defines them.

The two existing providers fail in opposite ways:

- The v1 chezmoi provider enforces the right semantics but speaks the wrong protocol, performs review inside the provider (against the v2 boundary), and reads judge config live mid-run — a known defect class.
- The v2 reference fixture speaks the right protocol but: `explore → design` is an unconditional allow, an omitted policy set silently turns every gate into a vacuous allow, evidence has no author or subject identity, and stale or self-superseded evidence still satisfies gates.

Also, the fixture's policies and evidence exist only as inline Rust helpers — nothing in the repo shows, as inspectable data, what a real policy config or evidence record looks like.

## 2. Outcome

A production-grade software-change provider lives in the loop-engine repo as the living reference workflow. When a run uses the shipped standard config:

- No progression past intent, design, or final validation without external review evidence for the configured axes.
- Obligations are explicit, frozen at run creation, and visible to a fresh actor without a provider call.
- Evidence that doesn't say who wrote it and what it reviewed doesn't count. When an artifact's declared revision changes, old evidence doesn't count.
- Malformed artifacts are denied by cheap deterministic checks, and that denial takes precedence over — and is unaffected by — any semantic evidence.
- A zero-obligation run is an explicit visible choice, never an accident.
- The semantic judgment stays external. The provider validates evidence conformance, never truth.
- Reviews are pragmatic: a denial must point at something that plausibly affects the change's success against its intent, not rubric pedantry.
- A change is not complete until the repository's authoritative documents — notably the repo-wide PRD — reasonably integrate what the change established. The change-scoped intent/PRD is an input to those documents, not a parallel truth left to drift.

## 3. Boundary (inherited, not restated)

The engine PRD fixes: `describe`/`evaluate` only; `describe` is input-independent and `start` performs no provider admission check; provider validates, never routes, never does the work; review is external and arrives as opaque context; core assigns context no truth or provenance; policies live in immutable initial input; the provider cannot write context or persistence; no engine policy model, review-result type, prompt generation, or review orchestration. Providers may read externally managed work, but no read is atomic with a transition commit and nothing is locked — residual accepted. Every provenance rule below is a provider convention over evidence *content* — presence and conformance checked deterministically, truth external.

## 4. Requirements

### Gates

- R1. Topology unchanged: the engine PRD's 9-state reference topology, no new edges.
- R2. All five checked boundaries carry policy-configurable semantic obligations: **intent** (`explore → design`), design review, plan review, implementation review, validation.
- R3. Schema checks run at two points: the `*-ready` transitions (and the intent boundary's deterministic half) check the artifact early, and **every semantic approval transition (`design-review → approved`, `plan-review → approved`, `implementation-review → approved`) and `validation → passed` rechecks the current subject artifact against its schema before evidence aggregation** — an artifact made malformed after its ready-check does not advance. The shipped standard and high-rigor configs include subject schemas for all five gates, so gates with no semantic axes still carry the deterministic check. A gate with no configured schema has **no structural check configured**, and the workflow's guidance says so. No transition implies checking it doesn't perform.
- R4. Deterministic precedence: when a gate carries both a schema check and semantic obligations, a schema failure denies with "not judged: fix shape first" feedback, and the denial is identical whether or not semantic evidence exists. Gate guidance instructs executors to run the shipped deterministic check before commissioning external review; the provider guarantees precedence, not that review effort was never spent.

### Artifacts and schemas

- R5. The run declares an artifact location as a run input. The provider reads authored artifacts from it for schema checks only — contained reads (no path escapes), never writes, never locks. An inaccessible or invalid artifact location is an evaluation error; an expected artifact that simply doesn't exist yet is a deny (work not yet authored), not an error.
- R6. **No artifact structure is embedded in the provider.** Schema checking is one generic behavior; the artifact schemas per gate arrive at runtime in the shipped config (R9), frozen per run like policies. The schema language is chosen at design (rec: a standard JSON Schema subset) and must express at least: required fields, types, closed field sets, non-empty strings and string arrays, and cross-artifact revision-linkage declarations ("this design must name the intent revision it was written against").
- R7. Artifacts carry an author-declared `revision` string and an author declaration (both required by the shipped schemas). Bumping the revision means "this changed materially — prior review no longer applies." Not bumping it asserts an edit was immaterial; that assertion is trusted (solo-operator tool). Accepted residuals, stated precisely: a material edit without a bump is undetected; an edit made *after* the final review evidence is appended is undetected even by the validation gate; and a no-op bump retires standing failing verdicts (R16) without any real change. All are claim-trust residuals of the same deliberate trade v1 made.
- R8. **Gate subjects.** Each gate binds evidence to one schema-checked subject artifact and its declared revision: intent gate → the intent artifact; design review → the design artifact; plan review → the plan artifact. Implementation review and validation bind to their gate's report artifact, which carries its own revision **and a declared coverage manifest**: one repository commit identity for covered repository state, plus declared revisions for any covered documents under the artifact location. Evidence binds to the report's revision; the manifest makes multi-artifact coverage explicit and inspectable. **The subject's author is whatever the subject artifact itself declares** (R7); R17's independence checks compare against that declaration.

### Policy and schema configuration

- R9. **No policy, schema, prompt, or artifact-shape content is baked into the provider.** The repo ships versioned config data files — `minimal`, `standard`, `high-rigor` — bundling review policies, artifact schemas, and example prompts. A caller passes one at run creation (bundled alongside the binary, usable as-is or edited). Policies and schemas live in initial input, frozen for the run, readable via `show`. The initial input is the obligation of record.
- R10. A run created without a `review_policies` key is not admitted anywhere (the engine has no admission check); instead, **every checked evaluation fails as an evaluation error**, beginning with `intent-ready`, with a diagnostic naming the shipped configs. An explicitly empty policy set is accepted and the handoff surface shows no review obligations. Malformed policies or schemas (bad shape, unknown gate, unparseable schema) fail closed with a diagnostic — never fewer obligations.
- R11. A policy entry may carry extra fields (notably `example_prompt`) that the provider ignores as opaque data. The example prompt informs the reviewer executor: it captures a known-good way to judge the axis, but the binding obligation is the policy itself, and an orchestrator may use the example verbatim, adapt it, or replace it.
- R12. **Profile contents are product.** The shipped configs carry exactly these axis sets (ids and independence counts are normative; descriptions, prompts, and schemas are config-owned and freely evolvable):

| Gate | minimal | standard | high-rigor |
|---|---|---|---|
| intent | — | solution-agnostic, outside-verifiable, scope-fenced, constraints-are-limits, problem-grounded | same as standard |
| design-review | — | intent-faithful, acceptance-covered, structural-not-procedural, mechanism-forced | + decisions-justified, risk-honest, mechanism-forced; N=2 |
| plan-review | — | — | task-sized, context-sufficient, done-observable, decision-free, design-faithful, dependencies-honest |
| implementation-review | — | — | tasks-actually-done, no-scope-creep, design-faithful-final |
| validation | intent-delivered | intent-delivered, docs-integrated | + requirement-proof-mapping; N=2 |

  Gates marked — carry schema checks (R3) and no semantic axes. N=2 applies to **every axis** in that high-rigor gate cell, not only the added ones. (The v1 checkpoint axes live at implementation review; there is no checkpoint state.)

### Evidence

- R13. Evidence satisfies an axis only if it carries: gate, axis id, verdict, findings, the author claim (structured: name + kind ∈ human/agent/script), the subject, the subject revision it reviewed, and the config version it was judged under. Exact field names are design freedom; this information set is product. The config version must exactly match the version string in the run's frozen initial input — never the currently shipped config file; a mismatch is a distinct stale-config diagnostic and denies like any unsatisfied obligation. Any change to a config's policies, schemas, or example prompts requires a version bump; bump correctness is caller-managed and undetectable (accepted claim-trust residual).
- R14. Nonconforming evidence never satisfies. A malformed record that can be associated with a configured axis of the gate under evaluation blocks that axis with a distinct diagnostic ("malformed evidence for axis X") until **any later conforming record for the same gate and axis** is appended — append order, author-independent, since a malformed record may lack an author. A record that cannot be associated (missing gate or axis id) is inert: it changes no verdict and is mentioned only when the evaluation already denies (an allow carries no feedback). Malformed records never poison other axes or gates.
- R15. Stale evidence never satisfies: evidence naming a revision other than the subject's current declared revision leaves the axis unsatisfied, and the denial names both revisions. Same mechanism as schema cross-artifact linkage — no extra machinery.
- R16. **Aggregation.** The supersession key is (axis, subject revision, author): each author's latest conforming verdict per axis per revision stands. An axis is satisfied when it has at least N distinct-author passing verdicts (N=1 unless the axis declares more) **and no standing failing verdict**. A fail is retired only by the same author's later pass or a subject revision bump — never outvoted, never overwritten by another author.
- R17. **Independence.** No verdict from the subject's declared author counts toward any axis. Where an axis declares N≥2, all N authors must be distinct and none may be the subject's author. The provider checks presence and distinctness of author claims; their truth is external.

### Failure and feedback

- R18. Policy rejection ≠ broken evaluation, throughout: unsatisfied obligations and schema failures deny with actionable feedback; malformed config, inaccessible artifact location, or provider incapacity is an evaluation error that never advances the run and never counts as satisfaction. An artifact that is present but unparseable is a deny (nothing is broken except the document), not an error.
- R19. Denials name, per schema check: each violated schema rule; and per axis: missing / failed (with findings) / malformed / stale (both revisions) / stale-config / unmet independence — plus prior denial lineage.
- R20. No waivers. Obligations can be satisfied or the run terminated, never reduced mid-run.

### Reviewer guidance

- R21. The shipped configs embed example prompts ported from the v1 rubric set for every axis in R12's table, plus a short reviewer-protocol doc (response format, adjudication rules, untrusted-material handling). Together these are the shipped reviewer contract; each config file carries a version, and evidence names it (R13). Orchestrators may adapt or replace prompts (R11); the A11 calibration obligation applies to the shipped defaults only.
- R22. **Materiality over pedantry.** Shipped example prompts must instruct reviewers to deny only for issues that plausibly affect the change's ability to succeed against its intent. They keep v1's anti-pedantry guards (silence is not a finding, bounded omission rules, do-not-invent-norms, no length/count proxies) and v1's non-waiver rule — scoped so that pedantry is not a finding, and a material finding cannot be waived.
- R23. Authoring-state instructions are static text pointing at the shipped templates and the run's frozen config; per-run obligations are read from `show` (initial input), never generated dynamically. The static guidance carries: intent — the authoring rules (problem, outcome, acceptance boundary, constraints, non-goals; no prescribed implementation unless externally mandated) and the target-laundering warning; design — structural shape, not a work schedule; plan — the task-packet shape: per-task objective, dependencies, source-of-truth references, deliverables, out-of-scope, validation, and handoff contract, as a dependency graph with contract gates before parallel fan-out; implement — doc integration is part of the change.

### Living reference

- R24. The provider registers and runs against the v2 engine as a normal external provider and is exercised by the repo's acceptance tests as the reference software-change workflow.
- R25. The shipped configs and at least one complete example evidence record are data artifacts consumed by the acceptance tests — the same files the docs show.
- R26. The repo ships artifact templates for each authoring state — intent, design, task packet/graph, and the implementation/validation report shapes (with coverage manifest) — with the task-packet template derived from the packet shape actually used to implement Loop Engine v2 (`loop-engine-v2-implementation-task-packets-amended.md`). The shipped schemas (R9) are the machine-checkable form of these templates; templates carry the human guidance, schemas carry the deterministic floor, and the example prompts reference both.
- R27. **Doc integration is part of the change.** The standard config's `docs-integrated` validation axis judges whether the repository's authoritative documents (the repo-wide PRD, and whatever else the change touched) reasonably integrate the change: no contradiction with delivered behavior, no orphaned change-PRD acting as a parallel source of truth. The task-packet template carries doc integration as an explicit deliverable. Same externality as every axis: reviewers judge it, the provider only checks the evidence. The example prompt notes that a completed policy-document audit run over the affected docs is acceptable evidence — composition with the document-audit workflow happens at the orchestration level, never as provider coupling; repo-wide doc health beyond this change's scope stays with that workflow on its own cadence.

## 5. Non-goals

- No engine/core/protocol changes; no new provider operations; no run-creation admission check.
- No topology changes: no new states, no `revise → explore`, no checkpoint state (D9 deferred; checkpoint axes ride implementation review).
- No provider-performed judgment, judge spawning, or model config in the provider.
- No prompt *generation* — shipped prompts are static data the provider ignores.
- No cryptographic provenance: author identity is a checked claim, not a verified one. No signing or allowlists.
- No content hashing, locking, or versioning of external work; artifact reads are non-atomic with transition commits. Revision strings are author claims; the silent-edit residuals in R7 are accepted by design.
- No mid-run waiver/amendment of obligations.
- No v1 migration. No automated calibration harness (A11 uses a documented manual procedure).

## 6. Acceptance criteria

Each provable by an executable test or scripted inspection; final validation maps requirement → proof (matrix is a validation-gate deliverable, not repeated here).

- A1. Run created with the shipped standard config: `intent-ready` denies until the intent artifact passes its schema AND conforming intent-axis evidence exists; obligations and schemas readable via `show` before any evaluation.
- A2. Run created without `review_policies`: creation succeeds; the first checked evaluation returns an evaluation error naming the shipped configs; the run does not advance. Run with explicitly empty policies AND no schemas: traverses on events alone, `show` shows no obligations.
- A3. Under the standard config no checked transition is an unconditional allow; transitions with no structural check configured are named as such in guidance.
- A4. Evidence missing a required field but attributable to a configured axis of the evaluated gate denies with a malformed-evidence diagnostic distinct from missing-evidence; an unattributable record changes no verdict and is noted only when the evaluation already denies; a malformed record for one axis never affects another axis or gate.
- A5. After a gate's subject artifact's declared revision changes, old passing evidence no longer satisfies; denial names both revisions; fresh evidence for the current revision satisfies.
- A6. High-rigor: an N=2 axis denies when the two passing claims share an author or when any counted claim's author equals the subject's declared author; allows with two distinct non-subject-author passes; a standing fail from a third author blocks allow until that author passes or the revision bumps.
- A7. Malformed policy config fails closed with a diagnostic; obligations never reduced by bad input.
- A8. Provider-level: within one subject revision, an author's later pass supersedes that author's earlier fail, and another author's fail is unaffected. Engine-level (separately asserted): `show` projects the latest evaluation per transition, later allow superseding earlier denials.
- A9. Evaluation errors (inaccessible artifact location, malformed config, provider incapacity) are distinguishable from denials in engine outcomes and never advance the run. Stale-config evidence is a denial (unsatisfied obligation), never an evaluation error.
- A10. Evidence carrying a config version other than the run's frozen version does not satisfy and yields the stale-config diagnostic (tested with a deliberately mismatched record).
- A11. **Futility test:** calibration fixtures include good-but-imperfect artifacts (minor blemishes, no material defect) that must PASS under the shipped example prompts, and materially defective artifacts that must FAIL — decided by a documented manual procedure (stated model, stated invocation, owner-attested results recorded with the fixtures). A shipped example-prompt change that flips a good-but-imperfect fixture to FAIL is a breaking change.
- A12. An artifact violating its configured schema denies with every violated rule named, and the denial is byte-identical whether or not semantic evidence exists in context; a subject made malformed after its ready-check denies at the subsequent approval transition; a gate with no configured schema is labeled accordingly in guidance; an unparseable shipped schema fails closed as an evaluation error.
- A13. The shipped standard config contains the `docs-integrated` validation axis and the axis sets of R12's table exactly (scripted); the shipped task-packet template contains the doc-integration deliverable (scripted); the report schemas require author, revision, and coverage-manifest fields (scripted).
- A14. `describe` output matches the engine PRD's reference software-change topology exactly (scripted comparison).
- A15. Each of the three shipped configs starts a run cleanly and its policies/schemas are readable via `show` (scripted).

## 7. Open questions (owner decides before design freeze)

- OQ-B. Schema language: standard JSON Schema subset vs a small custom declaration format. Rec: JSON Schema subset; custom only if revision-linkage rules don't express cleanly.
- OQ-D. Does this provider replace the fixture in the engine's reference-workflow acceptance tests or live alongside it? Either way, the engine PRD's reference-workflow text (which today names only four review gates) must be amended to name the intent boundary as a configurable review gate when this provider becomes the reference.

Resolved since v0.3: OQ-A (structured author claim — R13), OQ-C (gate subjects — R8), OQ-E (A11 manual owner-attested procedure).

## 8. Process

This PRD follows its own playbook. Round 1 (openai-codex/gpt-5.6-sol): Not ready, 6 blockers — all dispositioned and applied in v0.4. Round 2 (same model, fresh context): Not ready, 5 wording-level blockers (schema-recheck placement, N-scope, malformed recovery, stale-config classification, subject identity sources) — all applied in this draft, 14/17 round-1 fixes verified clean. Round 3 (same model, fresh context, narrow verification): 10/11 checklist items landed cleanly; one stale A4 clause found and corrected. Document verified internally coherent and engine-PRD-consistent. Owner froze 2026-08-11. Technical design may begin against this document; material changes reopen it as an explicit amendment, not a silent edit.

Amended 2026-08-12 (explicit amendment, owner-approved): R12 design-review row gains `mechanism-forced` in standard and high-rigor — the symmetric KISS/YAGNI sweep demonstrated direct gate yield during this provider's own design reviews; shipped bundles previously carried no design-simplicity axis. Config versions standard-3, high-rigor-3.

Amended 2026-08-12 (explicit amendment, owner-approved): the v0.1 repo-local-only distribution statement is superseded starting in v0.2.1 by standalone distribution: release archives provide the `loop-engine` and `software-change` binaries, and `software-change` embeds its shipped data and exposes `data-dump` to materialize the repository-relative data tree. (The v0.2.0 release workflow failed before publication because the required cargo-dist profile was missing; no binaries were published.) Repository checkout remains a supported development path. The existing stdin/stdout `describe`/`evaluate` wire contract is unchanged; no workflow, gate, policy, or other requirement changes are made.

Amended 2026-08-12 (explicit amendment, owner-approved): v0.2.2 is the contract-closure release. The public append CLI preserves caller-owned record IDs in both documented option forms; source-tree and packaged production journeys prove deterministic provider mechanics across separate processes; cargo-dist publication is dispatch-only and gates host on preflight, native macOS arm64/Linux x86_64 builds, and extracted archive smoke. Synthetic journey evidence proves shape, linkage, independence, aggregation, routing, and persistence only; semantic review remains external. Historical v0.2.0 and v0.2.1 tags remain immutable. An owner-created raw tag outside this private/free-repository workflow remains an explicit residual that repository automation cannot fully prevent.

## 9. Contract-closure amendment (v0.2.3 candidate)

The provider now exposes conventional `--help`/`-h` and `--version`/`-V` before stdin while preserving `describe`, `evaluate`, `data-dump`, and unsupported-argument behavior. Direct pushes to `main` dispatch read-only preflight at the pushed commit; generated release workflow ownership and publication topology remain unchanged.

A11 binds each existing calibration row to exact supplied bytes: shipped reviewer instruction, selected prompt/protocol/template/schema, subject and required predecessors, mapped fictional companions, and canonical request JSON. `input_sha256` is a mechanical test identity only. Existing owner observations were made against prior input bytes; changing supplied content requires documented fresh owner re-attestation and does not get automated by the provider. Evidence denial projection keeps current blockers in `details.diagnostics` and moves stale/stale-config records to `details.informational` without changing denial codes or envelopes.
