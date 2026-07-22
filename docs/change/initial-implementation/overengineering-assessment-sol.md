## 1. Executive summary

**Q1 — PLAN: OVER-ENGINEERED, 96% confidence. Q2 — CURRENT REPO: OVER-ENGINEERED, 99% confidence.** Product intent describes narrow local workflow coordinator for one user, one active state, executable providers, durable state, journal, and CLI. Plan expands this into **200 T-tasks, 26 V/F junction tasks, 5 governance-repair tasks, 16 decision gates, roughly 32 commit boundaries, per-operation facet manifests, blind reviews, range ledgers, immutable-SHA reports, and semantic-model publication gates**. Current tree contains **61,847 Rust LOC and 625 tests, yet shipping CLI contains 3 LOC and exposes no application operation**. Robust persistence, provider isolation, and atomicity work have value; sequencing and proof machinery do not deliver proportional user value.

## 2. Q1 — Plan verdict

### Verdict: **OVER-ENGINEERED — 96% confidence**

Plan is internally diligent and largely faithful to frozen contracts. Problem: frozen contracts themselves exceed product scale.

### Intent-to-process mismatch

Foundation says:

- local tool for one user and independent runs (`docs/intent.md`, **Purpose**);
- flat graph, one active state, explicit events, cycles, gates, state, journal (`docs/intent.md`, **MVP direction**);
- no multi-user coordination, scheduler, workers, variables, expression language, or broad platform (`docs/tenets.md`, **14. Focused core over broad platform**);
- layer count is contingent, not goal (`docs/architecture.md`, **Architectural rule**).

Plan responds with:

- **T001–T016:** 16 decisions before workspace code;
- **T017–T030:** 14 workspace/governance tasks before domain model;
- **R001–R005:** five extra governance-repair tasks;
- **T031–T119:** 89 domain/integration/persistence tasks before CLI work;
- **T120:** first real CLI startup task;
- **T146:** first public route;
- **T167–T184:** 18 aggregate acceptance tasks after every operation already closed its own mandatory facets;
- **T185–T190:** six more reference-acceptance/report tasks;
- **T191–T200:** ten evidence, documentation, quality, repository-policy, and closure tasks.

Runtime work and governance work have near-equal planning status. That is wrong priority for local `0.1.0` CLI.

### Ceremony beyond runtime need

#### 1. Publication machinery

T023–T029, R001–R005, T195–T200 build:

- versioned model-judge request/response schemas;
- parent-versioned rubrics;
- staged-tree and remote-range evaluation;
- detached candidate worktrees;
- migration rubric exceptions;
- credential-separated CI phases;
- branch-protection evidence and rollback artifacts;
- immutable-SHA acceptance reports.

`xtask/src/semantic_judge.rs:20-70` already contains frozen commit hashes, rubric digests, migration bases, section names, and bootstrap compatibility machinery. None improves workflow transition correctness. It enforces I47, but I47 is process policy elevated into product-level invariant.

#### 2. Range-ledger and junction bureaucracy

Plan adds V001–V013 and F001–F013 around T-ranges, plus boundary ledgers and exact path unions. Validation and repair cycles are sensible. Encoding each as paired plan entities with marker state, evidence directory, ledger membership, rerun protocol, and commit gate is not.

Visible maintenance failure already exists: `tasks.md` header says V005/F005 is next, while both markers are `[x]` and T120 is first open. Process structure creates stale process metadata before product has CLI.

#### 3. Duplicated verification layers

Each exposure task T146–T166 requires:

- route;
- core/driver/runtime catalogs;
- structured and human rendering;
- facet JSON;
- valid and invalid E2Es;
- trace proof;
- persistence proof;
- closure;
- docs;
- dedicated commit.

Then T167–T184 repeat families across outcomes, provider failures, graph semantics, evidence, lifecycle, compatibility, concurrency, migrations, corruption, export, traces, model tests, and exclusions. T185–T190 repeat reference behavior again. T191 wraps all proof into another report.

Coverage traceability is useful. Three nested proof layers are excessive.

#### 4. Rare-path hardening dominates MVP

Examples:

- HMAC-protected cursors and disable acknowledgements;
- ten-provider-call page ceilings;
- cross-process trace byte reservations;
- `RLIMIT_FSIZE` late-sink truthfulness tests;
- export staging, rename, parent `fsync`, crash recovery, hash verification;
- every SQLite write-boundary fault;
- every provider process/protocol failure repeated across multiple operations;
- corruption taxonomies covering malformed rows, sequence allocators, referential mismatches, future corrections, graph-digest mismatches.

These are defensible in isolation. Combined scope resembles mature infrastructure product, not first local CLI release.

### Ceremony pulling weight

Not all ceremony is waste:

- **T104–T119 persistence:** directly protects I12–I16 and I35–I36. Atomic state/journal/evidence and stale-provider-result rejection are core trust promises.
- **T084–T095 provider boundary:** arbitrary executable invocation, bounded streams, timeout, malformed output, exact gate verdicts, and no provider state authority deserve careful implementation.
- **Production CLI E2Es:** foundation explicitly makes CLI behavioral authority. Real subprocess, SQLite, restart, rejection, and trace tests are justified.
- **Three crates:** required by I22 and technically sensible.
- **Reference revision-cycle workflow:** proves cycles, gate ownership, evidence, handoff, and provider drift against real use.

Best plan portions protect authority boundaries. Worst portions prove proof machinery and publication ritual.

## 3. Q2 — Current repo verdict

### Verdict: **OVER-ENGINEERED — 99% confidence**

Current repo is robust substrate without product. Abstraction count alone is not alarming; proportionality and sequencing are.

### Volume

| Component | Rust LOC | Estimated production LOC | Estimated test LOC | Test/prod |
|---|---:|---:|---:|---:|
| `loop-engine-core` | 13,224 | 9,079 | 4,145 | 0.46 |
| `loop-engine-integrations` | 38,308 | 22,634 | 15,674 | 0.69 |
| `loop-engine-cli` | **3** | **3** | 0 | 0 |
| `xtask` | 10,312 | 6,313 | 3,999 | 0.63 |
| **Total** | **61,847** | **38,029** | **23,818** | **0.63** |

Additional metrics:

- **625** `#[test]` functions;
- **54** dedicated test `.rs` files across workspace, including xtask fixtures;
- **15** dedicated product-crate test files, plus extensive inline tests;
- **11** public traits;
- roughly **669** `pub`/`pub(crate)` declaration lines across core and integrations;
- product crates total **51,535 LOC**;
- CLI exposes no usable product behavior.

Testing ratio itself is not absurd for transactional/provider code. Timing is absurd: foundation says lower-level tests cannot establish behavioral completion, yet 625 lower-level tests exist before one production-driver route.

### Module shape

Largest files:

- `integrations/src/persistence/event_attempt.rs` — **3,371 LOC**;
- `integrations/src/persistence/provider_catalog.rs` — **2,867 LOC**;
- `integrations/src/persistence/run_mutations.rs` — **2,574 LOC**;
- `integrations/src/persistence/history.rs` — **2,410 LOC**;
- `core/src/operations/run_request.rs` — **2,366 LOC**;
- `integrations/src/persistence/corruption.rs` — **2,118 LOC**;
- `integrations/src/export/mod.rs` — **1,783 LOC**;
- `xtask/src/semantic_judge.rs` — **1,768 LOC**.

Top-level integration source:

- persistence: **25,352 LOC**;
- provider protocol: 2,471;
- export: 2,035;
- provider process: 1,056;
- trace: 995;
- configuration: 464.

Persistence is almost entire product. Some size comes from tests, but production portions remain huge: `event_attempt.rs` test module begins around line 1950; `run_request.rs` around line 1536; `corruption.rs` around line 2086.

`run_request.rs:25-66` defines a wide resolution taxonomy, then `execute` uses four capability type parameters, four closures/error parameters, and `#[allow(clippy::too_many_arguments)]` at `run_request.rs:68-100`. Complexity comes from preserving every intermediate classification and journal branch, not transition resolution itself.

`event_attempt.rs:1-60` imports authority, decision, evidence, journal, trace, commit-verification, and DTO concerns into one persistence unit. `provider_catalog.rs:20-40` adds HMAC cursor/ack domains and three collection protocols before basic local catalog behavior. `corruption.rs:30-119` defines 20 diagnostic codes and 20 semantic corruption categories before CLI can open a run.

### Abstractions

**11 traits are not excessive.** Time, IDs, digest, provider invoker, provider catalog, run reader/writer, event writer, export, and decision events correspond to real effect boundaries. Crate boundaries earn existence.

Over-engineering appears elsewhere:

- operation-specific command/result/journal types;
- duplicate mapping among core facts, persistence commands, JSON DTOs, rows, traces, and future CLI envelopes;
- exhaustive capability/result taxonomies before integrated use;
- cursor/ack integrity for same-user local database;
- separate compatibility and guidance attempt persistence;
- export crash protocol before basic inspection;
- publication infrastructure larger than CLI.

### Dependency shape

Direct dependencies remain restrained:

- core: 5 runtime, 1 dev;
- integrations: 12 runtime, 2 dev;
- CLI: 11 runtime, 2 dev;
- xtask: 7 runtime, 1 dev.

No async runtime, web stack, ORM, or service framework. Dependency selection is appropriately engineered. Custom code and process policy cause bloat.

### Effort trajectory

Git history:

- 16 commits spanning 2026-07-16 through 2026-07-22;
- latest persistence checkpoint alone: **36,586 insertions, 338 deletions**;
- historical churn:
  - integrations: 38,790 lines;
  - governance: 18,315;
  - core: 13,798;
  - docs: 13,092;
  - CLI: **33**.

Working tree is clean. This is not abandoned WIP causing misleading metrics. Effort overwhelmingly went into substrate, governance, and specifications. User-visible value remains zero.

## 4. Top 5 simplifications, ranked by effort saved

### 1. Defer secondary operation families

Defer:

- `provider.update`, rename, disable, restore;
- live guidance;
- explicit compatibility reports;
- audit export;
- label/correction polish.

Ship provider add/check plus run create/list/show/request/history first.

**Saved:** likely 20–30% remaining implementation/E2E effort.
**Lost:** full 21-operation D004 MVP; provider drift recovery UX; export; rich audit correction. Existing runs could still use stable configured providers and core transition behavior.

### 2. Remove per-operation provider-failure cross-product

Test subprocess transport failure classes once through provider adapter and one representative provider-dependent CLI operation. Give each other operation valid, semantic rejection, and operation-specific tests. Do not repeat timeout, signal, malformed JSON, wrong major, invalid UTF-8, oversized output, missing executable, and trace details across five operations.

**Saved:** major share of T148–T165 and T170/T182 work.
**Lost:** literal I30 facet-by-operation proof. Runtime behavior still shares same adapter, so engineering risk increase is modest.

### 3. Replace remaining microtask graph with vertical work packages

Collapse T120–T190 into roughly:

1. CLI composition/outcomes;
2. fixtures and harness;
3. provider/run core slice;
4. remaining catalog operations;
5. metadata/advisory/export;
6. integrated acceptance.

Run validation at each package end. Delete separate V/F IDs, range ledgers, intermediate candidate modes, and per-task commit ritual.

**Saved:** high coordination and review latency; less stale plan metadata.
**Lost:** exact task-level audit trail and narrow commit boundaries. Product semantics unchanged.

### 4. Remove semantic-model publication system from MVP critical path

Use deterministic quality gate plus ordinary human/code review. Keep semantic judge optional until project has multiple contributors or release risk.

**Saved:** remaining T190–T200 complexity plus ongoing push latency; avoids more work in already 10,312-LOC xtask.
**Lost:** I47 fail-closed semantic publication judgment, base-rubric evolution, immutable model verdict evidence. No runtime guarantee lost.

### 5. Simplify local-store hardening

Use signed or opaque cursors only where data is externally exposed; for local CLI, use simple stable keyset cursor. Replace paged disable ack protocol with one bounded warning plus explicit flag. Defer exhaustive corruption classification and export crash publication until real demand.

**Saved:** substantial catalog, persistence, trace, and E2E code.
**Lost:** same-user cursor tamper detection, exhaustive affected-run traversal binding, detailed rare-corruption diagnostics, strongest export crash guarantees.

## 5. Fastest credible path

Do **not** refactor completed substrate now. That burns more days without product.

Fast path:

1. Implement T120–T128 as one CLI substrate package: trace startup, parsing, composition, dispatch, JSON/human outcome, exits.
2. Build minimal scenario provider and process harness: valid graph, pass gate, fail gate, malformed provider.
3. Expose one coherent product slice:
   - `provider.add`
   - `provider.check`
   - `run.create`
   - `run.list`
   - `run.show`
   - `run.request`
   - `run.history`
4. Run one black-box sequence: register → create → inspect → rejected gate → successful transition → restart → history.
5. Publish as working alpha. Dogfood before implementing provider lifecycle, guidance, compatibility, metadata polish, export, and exhaustive failure matrix.

This path finally tests whether 51,535 product-crate LOC compose into useful behavior. It also attacks highest current risk: no evidence that independently tested core and integrations produce coherent CLI experience.

Of **81 remaining T-tasks**, roughly **45–53 task records (55–65%) can be collapsed** into larger work packages without dropping stated behavior. Because invariants themselves mandate exhaustive facets, reference coverage, traces, and publication judgment, only about **20–30% of remaining engineering effort can be removed without amending invariants**. If owner relaxes process/testing invariants I25–I31 and I47 while preserving runtime authority invariants I1–I24 and I32–I46, **40–55% of remaining effort** can credibly disappear.

Blunt assessment: project has built mature-system internals before validating first command. Stop expanding proof machinery. Wire product now.
