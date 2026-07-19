# Initial Implementation Change

**Status:** In progress

This change implements the complete `loop-engine` MVP defined by the foundation documents. The foundation is the sole requirements authority; this change introduces no OpenSpec process or artifacts.

## Authority

Every executor must read the foundation material relevant to its task before editing:

- [Product intent](../../intent.md)
- [Core tenets](../../tenets.md)
- [System invariants](../../invariants.md)
- [Code architecture](../../architecture.md)
- [Testing doctrine](../../testing.md)
- [Technology direction](../../technology.md)
- [Reference workflow](../../reference-workflow.md)
- [Interaction storyboards](../../ux-storyboards.md)

When this change conflicts with a settled foundation invariant, the foundation wins. Candidate choices remain unresolved until a Phase 0 decision task records owner approval and updates affected documentation.

## Artifacts

- [Decision gates](decisions.md) — implementation-stage choices, recommended defaults, and blockers.
- [Task list](tasks.md) — dependency-ordered task contracts for direct orchestrator execution.
- [Coverage map](coverage.md) — operation, invariant, facet, reference-workflow, and exclusion traceability.

## Intended result

A native Rust CLI with:

- exactly three product crates: `loop-engine-core`, `loop-engine-integrations`, and `loop-engine-cli`;
- a language-neutral executable-provider protocol with five narrow provider roles;
- a machine-local provider catalog and durable run catalog;
- validated per-run graph snapshots and authoritative current state;
- immutable ordered explanatory journal and append-only evidence;
- deterministic engine-controlled transitions and provider-defined gates;
- human and versioned structured CLI output with three outcomes;
- always-on secure per-invocation JSONL diagnostics;
- black-box production-CLI acceptance with real persistence and executable providers;
- mechanically closed operation/driver/E2E/trace coverage;
- generic per-commit semantic documentation judgment and exact-revision publication gates.

## Plan-publication precondition

Before T001, locally commit this four-file change pack plus its `docs/testing.md` facet clarification as one coherent candidate: `docs(plan): define initial implementation`. Stage those exact paths, run current documentation checks, attempt foundation-seed semantic judgment, and record unavailable/indeterminate only as local warning. Do not publish until T029 can judge every unpublished commit determinately. Implementation starts from clean working tree. Task-marker and coverage-key edits ride each later authorized boundary commit.

## Execution protocol

### Task states

Use these markers in `tasks.md`:

- `[ ]` not started
- `[~]` active; exactly one owner
- `[x]` task contract complete; for a T-task this means implementation deliverables are complete, while range validation remains pending until paired V/F junction closes
- `[!]` blocked; record blocker below the task

Do not mark a T-task complete because placeholder code exists. Mark it complete when its implementation, authored tests/fixtures, task-local acceptance criteria, and documentation obligations are complete. Commands explicitly deferred by the plan do not gate that T marker; paired V-task executes them and independently gates range acceptance, boundary commit, and later work.

### Direct execution and review handoff

Orchestrator executes T, V, and F tasks directly. T163 remains one uninterrupted multi-turn operation package, and same-owner atomic exposure groups remain indivisible; intermediate IDs stay `[~]` until final task closes group. Orchestrator stops rather than guesses when a stop condition fires, planned file contract no longer matches repository structure, or settled invariant appears contradictory.

Execution modes:

1. **T-task implementation:** author required code, tests, fixtures, and documentation; defer accumulated build/test/lint/closure commands to next V-task.
2. **V-task validation:** make no tracked edits; execute accumulated deterministic inventory and range-targeted suites; record clean result or exact findings.
3. **F-task repair:** batch all valid V findings into one coherent pass; rerun paired V-task before closure when repairs were non-empty.
4. **Blind review:** use fresh Fable and GPT-Sol subagents only for independent correctness/completeness judgment. Orchestrator validates accepted findings, applies repairs directly, and reruns V validation before another review round.

No T/V/F step commits independently. Orchestrator alone updates markers and range ledger, stages exact boundary union, invokes semantic/publication gates, commits, and pushes under boundary ritual. Review subagents never edit, commit, or push.

### Parallelism

Tasks may run in parallel only when:

- all dependencies are complete;
- file sets do not overlap;
- neither task changes public schemas, operation catalog, migrations, shared test harness, or foundation docs;
- neither task consumes output still being decided by another task.

Never parallelize migration edits, operation-catalog edits, protocol schema edits, CLI envelope edits, trace schema edits, or quality-gate edits.

### Test flow

For public behavior:

1. add or identify a black-box production-CLI scenario that fails because behavior is absent or wrong;
2. implement through production core, integrations, and CLI path;
3. make scenario pass against real executable provider and production persistence where applicable;
4. verify structured outcome and correlated trace;
5. add operation to runtime catalog only in the same checkpoint that closes its driver/E2E/trace proof.

Lower-level tests may guide implementation but never replace required CLI acceptance. No mock framework or mock-based behavioral test is permitted. T-task implementation authors scenarios as deliverables; execution and red/green confirmation occur in junction V-mode and boundary ritual, not after each individual T-task.

### Documentation coherence

Every commit must be independently coherent. Before committing:

- update affected foundation, protocol, CLI, schema, migration, provider-author, and operator documentation in the same commit;
- run deterministic documentation checks;
- run the configured semantic judge against exact parent-to-candidate diff using parent rubric;
- never defer documentation repair to a later commit.

Before pushing, every unpublished commit must receive determinate semantic-judge pass. Unavailable or indeterminate judge blocks publication.

### Public exposure rule

Internal implementation may land before a user-facing operation. A stable application operation is public only when the same publication checkpoint includes:

- core operation ID;
- production CLI route;
- structured and human rendering;
- required real integrations;
- valid-path E2E;
- applicable facet E2Es;
- operation envelope and trace observation;
- exact catalog-closure pass;
- same-commit documentation.

Do not add placeholder operations to runtime catalog. Every exposure task may register its route/catalog ID in a deliberately uncommitted candidate tree before running production-CLI E2Es and closure. “Private until green” means unpublished, not unreachable in candidate binary. Failed candidate removes/reworks registration before any commit.

Three same-owner atomic groups need each other for production-driver verification:

- WP-C3: T146–T147, `provider.add`/`provider.list`;
- WP-C4-C5A: T148–T152, `provider.check --active-runs` plus create/list/terminate/history;
- WP-C7A: T159–T160, evidence add/list.

Assign each full range to one fresh owner. Within group, `Depends` on prior group ID means prior substep acceptance rows pass while marker remains `[~]`; ordinary `[x]` dependency rule resumes outside group. First substep may create deliberately uncommitted checkpoint-working-tree route/catalog entries so CLI E2Es can run. Final named task closes every route/catalog/facet and reports entire group ready; orchestrator then marks range `[x]`. Never commit, parallelize, or hand off an intermediate tree.

### Commit ownership

Commit and push agency belongs only to orchestrator and is never delegated to blind review subagents. Every authorized boundary implicitly ends with `stop for orchestrator commit` under message/scope listed here; task prose saying candidate or authorized commit describes readiness, never permission. Foundation commit `7552af5968b4a2c10aefd01fbfa6c351817e1b8b` already consumed bootstrap publication exception and is parent seed rubric. T012 freezes how real judge reads that committed foundation rubric before focused files exist; T025 rubrics apply only to following commit. No implementation commit may push before T029, and first post-foundation push must show determinate judgment for every C0/C1 commit—no second bootstrap exception.

Candidate-commit ritual for every authorized boundary:

1. confirm working tree contains only completion-reported files from current boundary range; compare against range ledger, then stage exact union of those paths plus same-commit docs—never blind `git add -A`; boundary task's own Files field is not limit on accumulated range;
2. where a junction (V/F) gates this boundary, confirm its validation/fix cycle closed clean; run the canonical currently-implemented quality manifest and `git diff --cached --check` against staged candidate;
3. invoke real local judge on exact staged tree and parent rubric (`quality/semantic-judge/v1/build-exact-staged-request | quality/semantic-judge/v1/judge` for C0; `cargo run -p xtask -- judge --staged` after T024); determinate fail blocks, unavailable/indeterminate records warning locally;
4. inspect staged name/status and diff; orchestrator commits with `type(scope): checkpoint description`; record task IDs, commands, judge disposition, and documentation impact in commit body;
5. stop boundary work; orchestrator alone may invoke publication gate or push. Publication uses T028/T029 exact-commit range gate and requires determinate pass for every unpublished commit.

Authorized internal no-public-behavior boundaries: T052 `feat(core): establish domain model`; T083 `feat(core): add private operations`; T103 `feat(integrations): add provider config and trace substrate`; T119 `feat(persistence): add transactional state and journal`; T134 `feat(cli): add private driver and contracts`; T145 `test(e2e): establish black-box harness`; T167 `test(e2e): close operation catalogs`; T184 `test(e2e): close cross-operation acceptance`; T189 `test(reference): prove reference behaviors`; T191 `test(report): add immutable acceptance evidence`; T194 `docs: finalize operations and recovery`; T197 `chore(quality): finalize gate and flake audit`; T198 `chore(repo): enforce protected quality gate`.

Authorized public exposure boundaries: final atomic-group tasks T147 and T152, plus each independently closed exposure T153–T158 and T160–T166. Checkpoint/acceptance boundaries: T016, T030, T145, T147, T152, T154, T158, T162, T163, T165, T166, T190, T200. Intermediate group tasks never commit.

Boundary staging ranges are exhaustive and non-overlapping; they define commit scope, not same-owner execution. Each task completion report appends exact changed paths to current range ledger. Orchestrator commits only at range endpoint, stages ledger union, and resets ledger after commit:

| Boundary | Accumulated tasks | Commit message |
|---|---|---|
| T016 | T001–T016 | `docs: freeze implementation contracts` |
| T030 | T017–T030 | `build: establish workspace governance` |
| T052 | T031–T052 | `feat(core): establish domain model` |
| T083 | T053–T083 | `feat(core): add private operations` |
| T103 | T084–T103 | `feat(integrations): add provider config and trace substrate` |
| T119 | T104–T119 | `feat(persistence): add transactional state and journal` |
| T134 | T120–T134 | `feat(cli): add private driver and contracts` |
| T145 | T135–T145 | `test(e2e): establish black-box harness` |
| T147 | T146–T147 | `feat(provider): expose add and list` |
| T152 | T148–T152 | `feat(run): expose provider check and run foundation` |
| T153 | T153 | `feat(run): expose run.show` |
| T154 | T154 | `feat(run): expose run.graph` |
| T155 | T155 | `feat(provider): expose provider.update` |
| T156 | T156 | `feat(provider): expose provider.rename` |
| T157 | T157 | `feat(provider): expose provider.disable` |
| T158 | T158 | `feat(provider): expose provider.restore` |
| T160 | T159–T160 | `feat(evidence): expose add and list` |
| T161 | T161 | `feat(run): expose run.annotate` |
| T162 | T162 | `feat(run): expose run.label` |
| T163 | T163 | `feat(run): expose run.request` |
| T164 | T164 | `feat(run): expose run.guidance` |
| T165 | T165 | `feat(run): expose run.compatibility` |
| T166 | T166 | `feat(run): expose run.export` |
| T167 | T167 | `test(e2e): close operation catalogs` |
| T184 | T168–T184 | `test(e2e): close cross-operation acceptance` |
| T189 | T185–T189 | `test(reference): prove reference behaviors` |
| T190 | T190 | `test(reference): publish reference acceptance report` |
| T191 | T191 | `test(report): add immutable acceptance evidence` |
| T194 | T192–T194 | `docs: finalize operations and recovery` |
| T197 | T195–T197 | `chore(quality): finalize gate and flake audit` |
| T198 | T198 | `chore(repo): enforce protected quality gate` |
| T200 | T199–T200 | `chore(release): close initial implementation` |

Every endpoint has implicit task-local completion clause: `stop for orchestrator commit` using table message. A dirty path absent from ledger blocks commit; no later range absorbs it. Junction V/F governance tasks attach to the boundary rows named in `tasks.md`: the gated boundary commit waits for junction closure, F-task repair paths append to that boundary's ledger, and V-task evidence stays untracked and outside every ledger.

Revision-bound acceptance is two-stage: candidate commit contains implementation, task markers, stable evidence keys, generator/schema, and no self-SHA. After commit, local/CI runs generator against immutable SHA and stores untracked artifact/status under `<sha>/<report-key>`. Pass makes checkpoint accepted without tracked mutation. Failure explicitly authorizes a corrective commit limited to failed task/report scope; reopen affected marker, apply coherent fix/docs, rerun ritual, and generate new report. Tracked coverage never embeds own SHA or mutable artifact URL. T191 must commit generator before its post-commit report. After T197 closes, orchestrator commits T197, runs T028 publication range gate, and publishes candidate branch before executing T198 to configure/verify required GitHub check. T198 then commits repository-settings evidence. T200 follows same external-status rule.

## Review convergence severity contract

Blind reviewers report P0–P3, but gate fails only for foundation-backed P0/P1. Every gating finding must cite exact frozen foundation requirement or show objective task-graph impossibility; preferences, extra features, alternate designs, naming polish, duplicated wording, and advisory exactness cannot be promoted.

- **P0 — foundational safety/authority failure:** implementation as planned can corrupt or replace authoritative state, violate state+journal atomicity, execute providers during promised safe inspection, let providers select engine state, publish an unjudged commit, or otherwise defeat core product trust model. Fix requires foundation/architecture redesign or blocks all safe implementation.
- **P1 — mandatory-contract or executable-plan blocker:** plan cannot satisfy frozen foundation or a fresh owner cannot implement required behavior without inventing architecture. Examples: per-run compatibility suppresses provider-drift journal required by I13; disable can mutate before I40's full affected-run warning; an exposed operation lacks mandatory production-CLI/trace facet; dependency/file graph makes required build or verification impossible. Local typo is P1 only when no deterministic intended target exists and task must stop.
- **P2 — important but non-gating correction:** localized completeness, maintainability, or test-depth defect with clear repair that does not violate authority model or block implementation path.
- **P3 — advisory:** wording, naming, repetition, convenience, style, optional exact command spelling, or alternate design preference.

A reviewer returns `CLEARED` when no valid P0/P1 remains, even if P2/P3 are listed. `REQUEST CHANGES` requires at least one cited valid P0/P1. Missing root `plan.md` is not finding: this README is canonical entrypoint for intentional four-file pack. Reviewer-created `progress.md` is invalid evidence and must not be written.

## Publication checkpoints

| Checkpoint | Scope | Required condition |
|---|---|---|
| C0 | Decision freeze and judge contract | D001–D016 resolved; real local publication judge smoke-passes; CI provisioning owner named |
| C1 | Workspace and governance skeleton | Workspace builds; architecture and staged-tree gates pass; CLI exposes no application routes; runtime catalog arrives in C2 |
| C2 | Runtime substrate | IDs/time/config/trace/SQLite/outcome/E2E harness work without public operations |
| C3 | Provider catalog | `provider.add`, `provider.list` fully closed |
| C4 | Provider protocol check | `provider.check` protocol/graph/failure and registration-wide active-run facets close atomically with C5A at T152 |
| C5 | Run creation/read/lifecycle foundation | C5A create/list/terminate/history closes T152; show T153 and graph T154 complete checkpoint |
| C6 | Registration lifecycle | update/rename/disable/restore catalog semantics and stable run binding closed; current-config invocation closes in C8/C9 |
| C7 | Audit metadata | evidence add/list, annotate, and label fully closed across active/final/terminated runs |
| C8 | Event engine | `run.request` closed across gate, evidence, lifecycle, stale, and atomicity facets |
| C9 | Advisory compatibility | guidance and compatibility fully closed, including provider-declared guidance incompatibility |
| C10 | Audit export | read-only export fully closed across active/final/terminated runs |
| C11 | Reference acceptance | all 21 reference behaviors and model-based tests pass |
| C12 | Release-quality MVP | operation closure, hardening, docs, dependency policy, hooks, CI, and clean audit pass |

Each checkpoint may contain several commits, but every commit must remain buildable, documentation-coherent, and semantically judged. Internal preparatory commits must not claim unavailable public behavior.

## Global definition of done

This change is complete only when:

- every decision gate is resolved and reflected in foundation/contract docs;
- every task in `tasks.md` is `[x]`;
- every settled invariant I1–I47 maps to passing evidence in `coverage.md`;
- every final operation has valid path and all applicable behavioral facets;
- all four operation sets are equal: core catalog, CLI driver catalog, passing E2E observations, and trace observations;
- all 21 reference-workflow behaviors have explicit runtime evidence;
- production CLI, real persistence, and real executable provider fixtures are used for behavioral authority;
- required tests contain no mocks, ignored cases, quarantine, or known-failure allowance;
- migrations, corruption, rollback, overlap, crash, and trace rotation scenarios pass;
- canonical quality gate passes from a clean checkout;
- every unpublished commit receives determinate parent-rubric semantic-judge pass;
- authoritative CI passes and branch protection requires it;
- user/provider/operator documentation describes shipped behavior exactly;
- no excluded MVP feature or software-specific reference-workflow policy leaks into core.

## Non-goals

Implementation must not add:

- workflow YAML/JSON/TOML authoring DSL;
- agent invocation or primary-work execution;
- daemon, HTTP API, scheduler, worker queue, or async runtime;
- hierarchy, parallel regions, timers, compensation, or child workflows;
- mutable workflow-variable bag or expression language;
- event-sourced state reconstruction or deterministic replay promise;
- provider sandbox, registry, discovery scanner, installer, or trust database;
- work claims, leases, caller revision tokens, retry keys, or automatic provider retries;
- pause/resume, terminal reopen, individual-run deletion, import, restore, or cross-machine mobility;
- official provider SDK requirement;
- custom compiler plugin, pervasive trace-context plumbing, or per-function logging mandate;
- generic `util`, `common`, repository, or catch-all service abstractions.

## Change-control rule

If implementation discovers a missing requirement, contradiction, or necessary public-contract change:

1. stop affected task;
2. add or amend a decision gate;
3. update relevant foundation and coverage mapping;
4. obtain owner decision when choice changes scope or compatibility;
5. re-plan dependent tasks before implementation resumes.
