# Over-Engineering Assessment — Independent Read-Only Review

**Date:** 2026-07-21
**Assessor:** Independent read-only reviewer (no prior involvement in plan or code)
**Scope:** Plan (`docs/change/initial-implementation/`) and repository state as of commit `1fa8acf` plus ~3,552 uncommitted inserted lines.

---

## 1. Executive summary

**Q1 (the plan): OVER-ENGINEERED — high confidence.** The task plan is a well-constructed artifact, but it is a governance system wrapped around a product, not a plan for shipping one. **Q2 (the repo): OVER-ENGINEERED — medium-high confidence, with an important nuance.** The *architecture* of the code is genuinely disciplined and mostly appropriate (only 11 public traits in the whole product, clean dependency direction, no speculative generic repositories). What is over-engineered is the *allocation and volume* of effort: after roughly five days of nonstop work and ~61,000 lines of Rust, `crates/loop-engine-cli/src/main.rs` is **3 lines long**. There is no runnable CLI, no provider fixture, and zero end-to-end tests — which, under the project's own testing doctrine ("only black-box production-driver tests satisfy behavioral acceptance," testing.md), means the project has **zero behaviorally accepted operations** despite 613 test functions. Meanwhile ~10,300 lines of `xtask` process tooling (semantic judge, publication gates, hooks, quality manifests) — nearly the size of the entire core crate — exist to govern a codebase that cannot yet be run by a user. The single-user local-CLI intent stated in intent.md does not require this ratio of ceremony to product.

---

## 2. Q1 — The plan: OVER-ENGINEERED (high confidence)

### What the intent says this project is

intent.md is explicit about scale: *"MVP is local tool for one user operating one or several independent runs."* Tenet 14: *"Focused core over broad platform… Preserve extension seams without paying speculative complexity up front."* The MVP engine is deliberately narrow: one active state, explicit transitions, named gates, cycles, stored state, journal.

### What the plan actually is

- **231 execution units** for that MVP: T001–T200, plus V001–V013 junction validations, F001–F013 junction fixes, R001–R005 governance repairs (tasks.md §"Junction governance tasks").
- **112 lines of governance legalese before the first task** (tasks.md L1–112): path-alias grammar, four closure stages (`baseline`/`candidate`/`exposed`/`final`), junction command conventions, marker-authority rules, indivisible "atomic exposure group" rules.
- **A four-mode execution protocol** (README.md §"Execution protocol"): T-implementation, V-validation, F-repair, plus periodic *blind Fable + GPT-Sol subagent review rounds* — for a solo local project.
- **An LLM semantic judge gating every push** (D012, tenet 27, T023–T025, T028–T029, R001–R005): rubric schemas, versioned rubrics, exact remote-base-to-candidate-head aggregate judgment, fail-closed publication. This machinery already consumed five corrective tasks (R001–R005) to repair *itself*, and the most recent commit on the branch — `1fa8acf fix(quality): retry invalid judge citations` — is effort spent debugging the judge, not the engine.
- **Range ledgers** (`c5-range-ledger.txt`, 53 lines of file paths maintained by hand) and boundary-commit rituals with prescribed commit messages frozen into the plan (tasks.md junction table, e.g. T119 → `feat(persistence): add transactional state and journal`).
- **Facet-matrix exposure ceremony**: every one of the 21 operations gets a `quality/facets/v1/<op>.json` inventory, a prepare task, an expose task, and mechanical four-set closure (core = driver = route = E2E = trace). T163 (`run.request`) alone enumerates ~20 required facets — timeout/crash/nonzero/signal/malformed/wrong-major/invalid-UTF-8/oversized output, stale CAS, tombstoned providers — as *one indivisible multi-turn task*.
- **Phase 0 froze ~6,000 lines of contract documents** (operation-catalog, cli-contract, persistence.md, journal-contract, provider-protocol-v1, trace contract, bounds tables) across 16 decision tasks before a single line of product code. The docs tree is 11,362 lines — larger than the core crate's production code (8,793 lines).

### Where the ceremony pulls its weight

To be fair, several "heavy" elements are genuinely justified by the settled invariants and are good engineering:

- **No-mock, real-provider, real-SQLite E2E doctrine** (tenets 20–21) fits a product whose entire value is durable, auditable state. Correct call.
- **Atomic state+journal commit discipline** (I14) and the CAS/stale-evaluation design (T112–T113) is the actual product promise; the persistence care here is warranted.
- **Compiler-enforced crate/layer direction with an automated architecture check** (T021–T022, xtask/architecture.rs) is cheap and prevents real decay.
- **Freezing the provider protocol and CLI envelope before implementation** (D005, D006) is right for a contract-centric tool.
- **Mechanical operation-coverage closure** (tenet 22) is a good idea in principle — the over-engineering is in its four-stage ceremony, not its existence.

### Where the plan spends effort the intent does not require

- Nothing in intent.md/invariants.md requires an LLM judge, blind dual-model review rounds, junction V/F bureaucracy, range ledgers, or prescribed boundary commit messages. These derive from tenet 27 / the "Publication-checkpoint coherence" section of intent.md — i.e., the foundation *itself* bakes process ceremony into product intent. That is circular justification: the ceremony is "required" only because the same author wrote the requirement.
- The facet exhaustiveness (every process-failure mode × every provider-invoking operation, per T163–T165, T170) goes far beyond "fail closed and explain why" (tenet 17). One shared provider-failure test family would satisfy the invariant; the plan instead demands per-operation re-proof.
- 23 acceptance tasks (T167–T190) re-verify behavior the exposure tasks already had to prove, including a model-based black-box tester (T183) — a defensible luxury for a mature product, speculative for one with no CLI.

**Verdict: OVER-ENGINEERED.** Roughly a third of the plan's execution units (V/F junctions, R tasks, judge ritual, exposure ceremony, redundant acceptance re-proof) govern the work rather than do the work.

---

## 3. Q2 — The repo: OVER-ENGINEERED (medium-high confidence)

### Metrics

| Component | Files | Prod LOC | Test LOC | Notes |
|---|---|---|---|---|
| core/src | 64 | 8,793 | 3,142 inline (90 tests) | + tests/ 440 (10 tests) |
| integrations/src | 61 | 20,206 | 12,265 inline (253 tests) | + tests/ 5,727 (120 tests) |
| cli/src | 1 | **3** | 0 | empty `main` |
| xtask | 10 | 6,314 | 612 inline + 3,386 (140 tests) | dev tooling, non-shipping |
| test-support/ | — | — | — | **does not exist yet** |

~61k total Rust lines; 613 test functions; **0 E2E tests, 0 provider fixtures, 0 runnable operations**. Timeline: first commit 07-16, ~15 commits, large uncommitted persistence/export tranche in flight. Task progress: T001–T119 complete (~60%), but the completed 60% is entirely internal.

### What is *not* over-engineered

Credit where due — the feared abstraction disease is absent:

- **11 public traits in the entire product**, all in `core/src/capabilities/` (RunReader, RunWriter, EventAttemptWriter, ProviderInvoker, ProviderCatalog, TimeSource, IdGenerator, DigestComputer, DecisionEventSink, AuditExporter). No generic repository, no trait-object soup, no plugin framework. architecture.md's "no generic save/update repository exists" is honored in code.
- The three-crate split earns its existence: core genuinely contains no SQLite/process/CLI code; capability contracts use model types. Dependency direction is real, checked (xtask/architecture.rs, 1,070 lines), and clean.
- Tests overwhelmingly verify real behavior (transaction atomicity, CAS races in `tests/sqlite_overlap.rs` 1,292 lines, corruption handling, process timeouts) — not ceremony.

### What *is* over-engineered

1. **Effort inversion: zero user-visible value after ~5 days.** The project's own doctrine says only CLI-driven E2E counts, yet the CLI was scheduled *after* 20k lines of persistence/provider internals. All 613 tests are, by the project's own rules, non-authoritative supplements. If the doctrine is taken seriously, five days of work have produced nothing acceptable.
2. **Gold-plated internals ahead of any end-to-end path.** `persistence/event_attempt.rs` is 3,371 lines for one transaction type; `provider_catalog.rs` 2,858; `run_mutations.rs` 2,574; `history.rs` 2,410. `persistence/traced.rs` is a **1,516-line hand-rolled tracing decorator** for the persistence boundary. `corruption.rs` (2,118 lines) plus `tests/corruption.rs` (921 lines) implement corruption diagnostics for a single-user local SQLite file — before any user can create a run.
3. **`xtask` is 10,312 lines** — 87% the size of the entire core crate. `semantic_judge.rs` (1,768) + its tests (1,047) + `publication.rs` (891) + `quality.rs` (1,025) is ~4,700 lines of self-governance code, and it is already generating its own bug stream (`fix(quality): retry invalid judge citations`).
4. **Bounds ceremony.** `model/bounded.rs` freezes ~30 byte-budget constants (`JOURNAL_ENTRY_ENCODED_BYTES: 2_621_440`, per-field journal encoding budgets, argv element counts) with exact encoded-size accounting threaded through commands and persistence. Tenet 12 requires bounded capture; it does not require this granularity of budget bookkeeping for a tool whose only user is its author.
5. **Test-to-code ratio ~0.75 overall, ~0.9 in integrations** — high in absolute terms, and concentrated in exhaustive edge/failure enumeration (inline test modules of 12,265 lines in integrations) at layers the doctrine explicitly calls non-authoritative.

**Verdict: OVER-ENGINEERED** — not in abstraction (that part is disciplined) but in depth-first exhaustiveness, resilience engineering, and self-governance tooling, all sequenced before the product exists.

---

## 4. Top 5 concrete simplifications (ranked by effort saved)

1. **Demote the semantic-judge publication gate to advisory, or delete it.** Saves: ongoing maintenance of ~4,700 lines of judge/publication/quality xtask code, per-push LLM ritual, rubric versioning, and the recurring judge-debugging tax (R001–R005, `1fa8acf`). Keep the deterministic docs-check (T026) and CI quality gate. **Lost:** automated semantic doc-coherence judgment at each push — replaced by the owner reading their own diff, which for a solo project is what happens anyway.
2. **Collapse V/F junctions, blind review rounds, range ledgers, and boundary rituals into "run `cargo run -p xtask -- quality`, fix, commit."** Saves ~26 governance execution units (V005–V013, F005–F013, review rounds) and continuous marker/ledger bookkeeping. **Lost:** the forensic evidence trail under `target/junction-evidence/` and staged-repair discipline — low value when one person is orchestrator, implementer, and owner.
3. **Collapse the exposure waves T146–T166 (21 prepare/expose tasks, per-op facet JSONs, candidate/exposed/final closure stages) into: wire all CLI handlers (one task per command family), then grow one E2E suite.** Saves ~15–18 tasks and the per-operation atomic-publication ceremony. **Lost:** the guarantee that each operation becomes public only in a checkpoint that includes its full facet proof; retained: the simpler invariant that the final tip has every op covered.
4. **Trim the cross-operation acceptance phase T167–T184 (18 tasks) to ~5:** operation closure check, one shared provider-failure-family suite (reused across ops instead of re-proven per op), atomicity fault injection, lifecycle family, trace contract. Drop T183 (model-based black-box tester). **Lost:** per-operation exhaustive failure-mode re-proof and combinatorial exploration — the shared family still exercises every failure mode once through the production path, satisfying tenet 17.
5. **In code: cap resilience depth.** Slim corruption diagnostics (~3,000 lines incl. tests) to "detect, report, refuse to proceed"; replace the 1,516-line `persistence/traced.rs` decorator with trace calls at the ~10 actual command entry points; defer trace rotation refinements. **Lost:** granular corruption forensics and some trace uniformity — none of it required by I15/I35, which explicitly disclaim completeness.

---

## 5. Candid assessment: fastest credible path to a working product

**Where you actually are:** the hard, genuinely valuable 60% is done — model, decision semantics, provider subprocess protocol, and (uncommitted) transactional persistence are real and well-tested at their level. You are one crate away from a product. The danger is not the remaining code; it is the remaining *ceremony*, which as planned is ~81 T-tasks plus 9 V/F junctions where most of the engineering risk is already behind you.

**Fastest credible path (est. 2–3 focused days, not weeks):**

1. Run the accumulated V005 inventory once, fix, commit the persistence tranche. (½ day)
2. Build the CLI as one continuous effort — args, composition root, dispatcher, renderers, handlers for all 21 ops (T120–T134 collapsed to ~4 work items). The operations already exist; this is wiring. (1 day)
3. Build *one* scenario provider fixture with configurable graph/gate/failure modes and the E2E sandbox (T135–T145 collapsed to ~3 items; defer process-helpers and the invocation-ledger barrier machinery until a test actually needs them). (½–1 day)
4. Write E2Es per operation covering the three outcome classes (completed / rejected / errored) plus one shared provider-failure family and one atomicity fault-injection test. Expose everything in a small number of checkpoints, not 21.
5. Reference-workflow provider and its 21 behaviors (T185–T190) *after* the tool works — as validation, compressed to 2–3 items.

**What fraction of remaining planned tasks can be cut or collapsed without violating settled invariants?** Roughly **50–60%**. The invariants (I1–I35) constrain *engine behavior* — atomicity, gate authority, journal immutability, fail-closed outcomes — and are almost entirely satisfied by code already written plus the CLI/E2E work above. What gets cut is governance (V/F/judge/ledger), per-operation ceremony, and redundant re-proof. Two honest caveats: (a) cutting the judge ritual and facet-closure stages requires amending tenet 27 / testing.md's facet doctrine — the owner owns those documents and wrote them days ago; amending them is legitimate, not cheating; (b) tenet 22's mechanical closure is worth keeping in its simple `exposed` form — it is cheap and catches real gaps.

**Bottom line:** the codebase is not the problem; the operating system built around it is. The plan optimizes for provable process integrity on a project whose stated intent is a focused local tool for one user. Ship the CLI, keep the invariants, and let the ceremony go.
