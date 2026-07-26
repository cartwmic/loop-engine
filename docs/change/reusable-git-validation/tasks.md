# Reusable Git Validation Task List

**Status:** Proposed

Follow [README.md](README.md). This plan changes validation only; it does not create OpenSpec artifacts or alter loop-engine product behavior.

## Execution protocol

Task markers:

- `[ ]` not started
- `[~]` active; exactly one implementation owner
- `[x]` complete
- `[!]` blocked; record blocker under task

Task classes:

- **Implementation:** one fresh subagent owns one cohesive capability, focused tests, and handoff. It stops after that task.
- **Package gate:** orchestrator validates cumulative package behavior. Fresh blind reviewers do not edit implementation.
- **Closeout:** orchestrator owns repository-wide evidence and final disposition.

Rules:

- Complete dependencies and read their handoffs before starting a task.
- `Read first` is minimum context, not an allowlist. Inspect callers, dependencies, fixtures, and adjacent tests as needed.
- `Expected touch points` predicts likely edits but is not an exclusive file boundary. Escalate before expanding product or design scope.
- Keep one implementation owner across a task. Shared config, result, and report types change only in their owning task or through an explicit handoff amendment.
- Add failing contract tests before replacing behavior.
- Build final-state code directly. Do not add compatibility dispatch, migration inventory, rollout scaffolding, or rollback machinery.
- Intermediate tasks need focused proof, not an independently releasable old/new hybrid. Each package gate requires its cumulative final capability to be complete and green.
- Use `/usr/bin/git` for authoritative Git operations and remove `RUSTUP_TOOLCHAIN` from project check environments.
- Do not commit or push unless owner separately authorizes it. Git is the rollback mechanism; this plan does not rehearse restoration.
- Record any accepted design deviation in README before implementation proceeds.
- Orchestrator alone updates task markers; implementation/reviewer agents report status in handoffs and never edit marker lines.
- Every task writes its durable handoff beneath Git common directory at `loop-engine/change/reusable-git-validation/handoffs/<task-id>.md`; resolve common directory with `/usr/bin/git rev-parse --git-common-dir`. Create parent directories as needed and never rely on chat-only state.

Every implementation handoff records:

```text
Task:
Changed paths:
Contract/API produced for dependents:
Focused commands and results:
Additional files read:
Deleted superseded paths:
Deviations or unresolved risks:
```

Every package-gate handoff records:

```text
Gate task:
Accepted candidate tree:
Covered task IDs and changed paths:
Deterministic commands and results:
Blind-review disposition and evidence location:
Unresolved P2 findings:
```

## Validation cadence

### Task-local gate

Implementation owner runs:

1. `git diff --check`;
2. `env -u RUSTUP_TOOLCHAIN cargo fmt --all --check` when Rust changed;
3. task's named focused tests;
4. narrow check/clippy only for crates or targets changed.

After repair, rerun failed focused checks first. Do not run repository-wide gates for comments, fixtures, or isolated test corrections unless affected behavior crosses package boundaries.

### Package deterministic gate

Orchestrator runs cumulative gates at T005, T009, T014, and T017.

- T005 validates shared config/process/candidate/scheduler contracts.
- T009 validates final deterministic policy and exact staged hook.
- T014 validates semantic evidence, approval, aggregate publication, and local hooks.
- T017 validates final CI, deletion, documentation, supported platforms, and reduction target.

Each package starts with all focused targets owned by its tasks. Run full workspace check/test/clippy whenever package changes active hooks, manifest execution, publication authority, shared schemas, or removes code. Failed package gate returns findings to a narrow repair owner; after repair, rerun affected focused targets and then package gate once.

### Semantic review gate

Semantic review starts only after cumulative deterministic gate is green. Review cumulative package diff, not isolated task fragments.

Run fresh blind review at:

- T005: generic/project-policy separation, candidate isolation, and process safety;
- T009: final deterministic policy, retired-validator replacement, and exact staged behavior;
- T014: semantic fan-out, evidence integrity, approval non-bypass, and aggregate publication;
- T017: final CI equivalence, removal completeness, documentation, and architecture/size conformance.

Rerun semantic review only when repair changes reviewed behavior, contract, architecture, or evidence. Typographical and mechanically equivalent fixes need focused deterministic validation only. Reviewer output classifies blockers, majors, and minors with file/line evidence; gate remains blocked on unresolved blocker or major.

### Focused targets

```text
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test config
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test process
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test candidate
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test quality
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test workspace_architecture
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test semantic_judge
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test semantic_judge_schema
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test report
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test validation_commands
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test publication
env -u RUSTUP_TOOLCHAIN cargo test -p xtask --test hooks
env -u RUSTUP_TOOLCHAIN cargo test -p loop-engine-cli --test driver_catalog
```

Full deterministic gate means complete final manifest policy against exact candidate plus:

```text
git diff --check
env -u RUSTUP_TOOLCHAIN cargo fmt --all --check
env -u RUSTUP_TOOLCHAIN cargo check --workspace --locked
env -u RUSTUP_TOOLCHAIN cargo test --workspace --locked
env -u RUSTUP_TOOLCHAIN cargo clippy --workspace --all-targets -- -D warnings
```

After T008, exact staged acceptance uses `env -u RUSTUP_TOOLCHAIN cargo xtask validate --staged`. After T012, publication fixtures pipe canonical update lines into `cargo xtask validate --publication --updates-stdin`. After T015, CI fixtures also exercise `--ci-event <path>`.

## Dependency graph

```text
T001 ─ T002 ─ T003 ─ T004 ─ T005
                           └─ T006 ─ T007 ─ T008 ─ T009
                                                         └─ T010 ─ T011 ─ T012 ─ T013 ─ T014
                                                                                               └─ T015 ─ T016 ─ T017
```

Task boundaries follow final capabilities, not old/new migration stages. Shared-file overlap or an amended shared contract serializes affected work; no blanket serialization applies.

## Package A — Generic runner foundation

### T001 [ ] Define manifest v2 and binding contract

- **Class:** Implementation.
- **Depends:** none.
- **Read first:** README sections `Architecture boundary`, `Typed manifest`, and `Deterministic suite`; current manifest, quality runner, CLI dispatch, and manifest tests.
- **Settled constraints:** typed TOML argv; fixed placeholders; unknown fields fail closed; candidate policy applies immediately; generic types contain no project command IDs.
- **Expected touch points:** `quality/validation/v2/`, `xtask/src/config.rs`, `xtask/tests/config.rs`, config fixtures, `.cargo/config.toml` only for `cargo xtask` alias.
- **Deliver:** schema and immutable parser types for runner inputs, prerequisites, checks, phases, scopes, argv, cwd, timeout, output bounds, environment, scratch/cache/target placeholders, semantic axes, and coherence; one API computes SHA-256 of exact manifest bytes and path-sorted exact rubric bytes. Deterministic-only mode permits absent semantic configuration; publication/advisory mode requires complete semantic configuration.
- **Focused gate:** full/minimal manifests, exact-byte digest goldens, path ordering, unknown keys/values, duplicate IDs, invalid bounds/environment/placeholders, and path escape.
- **Handoff:** document public config types consumed by T002/T004/T010 and binding API consumed by T011/T012.
- **Done when:** fixture manifests parse without runner enums or project-specific branches.

### T002 [ ] Implement direct typed process executor

- **Class:** Implementation.
- **Depends:** T001.
- **Read first:** README sections `Architecture boundary`, `Typed manifest`, `Deterministic suite`, `Semantic topology`, and `Failure behavior`; T001 handoff; existing process execution and semantic fixtures.
- **Settled constraints:** no shell; candidate-root cwd; inherited environment then typed set/unset; lossless UTF-8/base64 evidence; per-stream limits; Unix process-group termination.
- **Expected touch points:** `xtask/src/process.rs`, `xtask/tests/process.rs`, process fixture executables.
- **Deliver:** project-neutral executor and typed outcome API covering timeout, signal, spawn, output-limit, duration, and cleanup evidence. Spawn returns an owned running-process handle with await plus idempotent external process-group termination so concurrent schedulers can cancel siblings without duplicating process mechanics.
- **Focused gate:** exact argv, empty/space values, environment precedence, missing/nonzero/signal/timeout outcomes, child-tree termination, externally triggered sibling cancellation and cleanup, repeated cancellation, output truncation, invalid UTF-8 round-trip, stream separation, and cwd escape.
- **Handoff:** document executor and cancellation-handle contracts consumed by T003/T004/T010.
- **Done when:** executor has no Cargo, Git, loop-engine path, or check-ID branches.

### T003 [ ] Implement exact Git candidate materialization

- **Class:** Implementation.
- **Depends:** T002.
- **Read first:** README sections `Candidate model`, `Typed manifest`, and `Failure behavior`; T002 handoff; current hooks, publication code, and candidate fixtures.
- **Settled constraints:** index-derived pre-commit tree; missing `HEAD` becomes empty-tree base; temporary Git index for revisions; explicit Git directory; candidate-only cwd; safe internal symlinks; external writable auxiliaries.
- **Expected touch points:** `xtask/src/git.rs`, `xtask/src/candidate.rs`, `xtask/tests/candidate.rs`, candidate fixtures.
- **Deliver:** repository/common-dir resolution, born/unborn staged and revision candidates, runner-input parity, read-only source, distinct writable scratch/cache/target descriptors, cleanup guard, and reusable `verify_unchanged` API that recomputes source tree/content/modes against bound `candidate_tree` without writing candidate source.
- **Focused gate:** unborn HEAD, unstaged/untracked exclusion, deletion/rename/mode/symlink changes, escaping symlinks, unusual filenames, non-UTF-8 rejection, interrupted cleanup, source-write rejection, writable auxiliary isolation, initial tree identity, and post-materialization content/mode mutation detection.
- **Handoff:** expose candidate/Git-common-dir descriptor and cleanup API consumed by T004/T008/T011/T012.
- **Done when:** candidate descriptor proves exact tree, read-only source, parity, safe symlinks, and cleanup.
- **Stop:** implementation copies current worktree as candidate source.

### T004 [ ] Implement generic deterministic scheduler and evidence

- **Class:** Implementation.
- **Depends:** T001, T002, T003.
- **Read first:** README sections `Typed manifest`, `Deterministic suite`, and `Evaluation and publication records`; T001–T003 handoffs; current quality runner and fixtures.
- **Settled constraints:** sequential manifest order; collect every failure; repository/changed-files scopes; stable distinct path argv; candidate binding; no hardcoded command dispatch.
- **Expected touch points:** `xtask/src/quality.rs`, `xtask/tests/quality.rs`, quality fixtures, plus `xtask/src/lib.rs`, `xtask/src/hooks.rs`, `xtask/src/publication.rs`, and their tests only where required to remove or adapt callers of the replaced API.
- **Deliver:** scheduler, prerequisite probes, placeholder expansion, changed-path delivery, all-failure aggregation, and machine-readable deterministic records using T002 outcomes. Invoke T003 `verify_unchanged` after every prerequisite/check process and again before final result derivation; source mismatch blocks remaining commands and fails closed. Port final-neutral callers and remove obsolete dispatch entries so workspace compiles; do not add compatibility dispatch or successful stubs. T008 and T012 own final hook/publication commands.
- **Focused gate:** ordering, phase filtering, multiple failures, empty changed set, unusual paths, writable-root expansion, probe/version failures without install, pre-spawn rejection, exact evidence, and a zero-exit command that mutates candidate content or mode then blocks following commands.
- **Handoff:** record scheduler API and result types consumed by T007/T008/T010–T012.
- **Done when:** arbitrary fixture manifests run without new Rust variants and cannot observe real-worktree source.

### T005 [ ] Gate generic runner foundation

- **Class:** Package gate.
- **Depends:** T004.
- **Read first:** T001–T004 handoffs and cumulative diff; README architecture, candidate, manifest, and deterministic sections.
- **Deterministic gate:** config, process, candidate, and quality focused tests; workspace check/test/clippy; `cargo xtask --help`.
- **Semantic gate:** fresh blind review of generic/project-policy separation, candidate-only observation, process safety, API cohesion, and accidental framework growth.
- **Handoff:** record accepted T001–T004 candidate tree, deterministic results, semantic disposition, and unresolved P2 findings.
- **Done when:** deterministic gate passes and no blocker/major semantic finding remains.
- **Stop:** candidate isolation is unproven or generic code embeds repository policy.

## Package B — Final deterministic policy and staged validation

### T006 [ ] Rehome objective and semantic policy

- **Class:** Implementation.
- **Depends:** T005.
- **Read first:** README sections `Ordinary tests own`, `Semantic rubrics own`, `Architecture validation`, and `Removal scope`; current architecture/dependency/documentation/operation/acceptance validators, focused rubrics, `deny.toml`, driver catalog, workspace manifests, E2E/trace support.
- **Settled constraints:** ordinary tools/tests own objective invariants; rubrics own judgment; initial-release evidence generation is not evergreen; do not recreate custom scanners under new names.
- **Expected touch points:** `xtask/tests/workspace_architecture.rs`, metadata fixtures, `crates/loop-engine-cli/tests/driver_catalog.rs`, directly affected E2E/trace tests, focused rubrics, and T006 handoff.
- **Deliver:** final tests for inward product-crate dependency direction and sole CLI binary; exact operation catalog/driver/route/E2E/trace/facet equality; preserve existing schema/protocol product tests without auditing every product contract; explicit final rubric text for documentation, observability, architecture, behavioral evidence, and cross-axis coherence. Do not reproduce retired scanner policy not required by these named final contracts.
- **Focused gate:** workspace architecture pass/fail fixtures; driver-catalog and directly affected E2E/trace targets; existing schema/protocol targets; rubric-content assertions covering all five named semantic owners.
- **Handoff:** record final test and rubric contracts consumed by T007/T010/T015; do not create an old-to-new inventory.
- **Done when:** named objective contracts have direct executable coverage, all five semantic owners have final rubric text, and no replacement expands product behavior.
- **Stop:** a named contract lacks coverage or replacement recreates a source scanner.

### T007 [ ] Install final deterministic manifest and delete superseded validators

- **Class:** Implementation.
- **Depends:** T004, T006.
- **Read first:** README sections `Typed manifest`, `Deterministic suite`, `Removal scope`, and `Dependency reduction target`; T004/T006 handoffs; manifest, quality dispatch, retired validator modules/tests/fixtures, `deny.toml`, provider commands, Go example, and `xtask/Cargo.toml`.
- **Settled constraints:** one schema-v2 deterministic registry; complete pre-commit/publication suite; cargo-deny 0.20.2; Go 1.26.5 through mise with auto-install disabled; no project command enum.
- **Expected touch points:** `quality/manifest.toml`, manifest docs, quality/config tests, hardcoded quality dispatch, retired deterministic validator modules/tests/fixtures, `xtask/src/lib.rs`, `xtask/src/hooks.rs`, `xtask/src/publication.rs` for caller removal only, `xtask/Cargo.toml`, and lockfile.
- **Deliver:** final typed manifest and aggregate diagnostics; remove custom documentation, architecture, dependency, operation-coverage, acceptance-report, parent/bootstrap, and hardcoded quality paths superseded by README final contracts; remove every in-tree reference to deleted APIs and all dead direct dependencies.
- **Focused gate:** exact prerequisite/argv/environment golden; complete read-only candidate suite; all T006 final tests and rubric assertions pass; searches prove no active command/config/test references deleted paths; workspace check keeps caller removal honest.
- **Handoff:** record final deterministic command inventory, deleted paths/dependencies, and APIs consumed by T008/T009.
- **Done when:** manifest is sole deterministic policy registry, final README-owned tests/rubrics are green, and workspace compiles without deleted APIs.
- **Stop:** active code references deleted path or generic runner gains project-specific dispatch.

### T008 [ ] Implement exact staged command and pre-commit hook

- **Class:** Implementation.
- **Depends:** T007.
- **Read first:** README sections `Candidate model` (`Pre-commit`) and `Hooks and installation`; T003/T004/T007 handoffs; current pre-commit hook, CLI parsing, and hook fixtures.
- **Settled constraints:** tiny tracked adapter; full deterministic suite; no semantic invocation; no source rewrite/staging; unborn behavior explicit.
- **Expected touch points:** `xtask/src/hooks.rs`, CLI dispatch, `.githooks/pre-commit`, hook/candidate tests.
- **Deliver:** stable `cargo xtask validate --staged` and final pre-commit adapter over v2 candidate/scheduler APIs.
- **Focused gate:** contamination, unborn repository, staged deletion, failure aggregation, signal cleanup, exit propagation, exact fixture-index acceptance, and staged candidate mutation detected before subsequent command/final verdict.
- **Handoff:** record exact staged command/hook contract consumed by T009/T012.
- **Done when:** pre-commit validates only index-derived source through final deterministic manifest.

### T009 [ ] Gate final deterministic and staged stack

- **Class:** Package gate.
- **Depends:** T008.
- **Read first:** T006–T008 handoffs and cumulative diff; README candidate, deterministic, architecture, and removal sections.
- **Deterministic gate:** workspace-architecture, driver-catalog, config, quality, candidate, and hooks targets; exact contamination matrix; full deterministic gate through `cargo xtask validate --staged`; searches for retired deterministic paths.
- **Semantic gate:** fresh blind review of policy/mechanics separation, invariant preservation, exact-index proof, read-only behavior, prerequisite behavior, and deletion completeness.
- **Handoff:** record accepted T006–T008 candidate tree, deterministic results, semantic disposition, and unresolved P2 findings.
- **Done when:** final deterministic policy and staged hook are green with no blocker/major finding.
- **Stop:** any check observes real-worktree source or retired deterministic code remains active.

## Package C — Semantic evidence and local publication

### T010 [ ] Implement final semantic contract and pipeline

- **Class:** Implementation.
- **Depends:** T009.
- **Read first:** README sections `Semantic topology` and `Architecture boundary`; T001/T002/T004/T006/T009 handoffs; current semantic contracts, focused rubrics, manifest, runner, and schema tests.
- **Settled constraints:** language-neutral stdin/stdout; exactly four axes; concurrent fan-out; distinct scratch roots; one correction attempt; coherence last; no coherence upgrade; Rust derives disposition; one manifest registry.
- **Expected touch points:** `quality/semantic-judge/v2/`, semantic runner/tests, `quality/manifest.toml` semantic block, and fixture judges. T006-owned rubric content is read-only.
- **Deliver:** request/response schemas, generic adapter/wrapper, v2 protocol text, normalized statuses, fan-out/correction/coherence pipeline, and mechanical `pass`/`semantic_block` result API. Rubric content remains owned by T006.
- **Focused gate:** schema/status matrix; correction; timeout/adapter failure; revision mismatch; citation/request-kind rejection; concurrency and scratch isolation; rubric isolation; order stability; missing/duplicate results; coherence monotonicity; candidate-root cwd and typed environment; candidate mutation by axis/correction terminates in-flight siblings, suppresses further children, and synthesizes complete unavailable results; coherence mutation forces semantic block.
- **Handoff:** document semantic process/result contract consumed by T011/T012.
- **Done when:** four configured axes always produce four normalized records and coherence cannot erase any non-pass.
- **Stop:** semantic executable chooses gate decision or duplicate registry remains active.

### T011 [ ] Implement canonical evidence, advisory review, and approval

- **Class:** Implementation.
- **Depends:** T010.
- **Read first:** README sections `Evaluation and publication records`, `Semantic topology`, and `Owner approval`; T001/T003/T004/T010 handoffs; existing storage, CLI dispatch, and Git-common-dir tests.
- **Settled constraints:** canonical UTF-8 JSON; SHA-256 external IDs; immutable atomic records; exact binding; UUIDv7 approvals; deterministic failure never approvable; advisory writes evaluation only.
- **Expected touch points:** `quality/publication-report/v1/`, `xtask/src/report.rs`, CLI validation service/dispatch, report and validation-command tests, canonical fixtures.
- **Deliver:** evaluation/approval/attempt schemas and store; exact-byte manifest/rubric binding; corruption-checked reads/writes; `validate --semantic --base ... --candidate ...`; `validation approve --report ... --reason ...`; immutable distinct approvals with collision retry.
- **Focused gate:** record schema/digest/nullability/corruption/round-trip; exact binding goldens; advisory candidate=HEAD with arbitrary base and complete publication phase; pre-commit-only exclusion; advisory success/block/deterministic-fail without attempt/approval lookup; approval reason/status/binding/corruption rejection; repeated approval uniqueness.
- **Handoff:** document store/query constructors, advisory contract, and approval-binding predicate consumed by T012.
- **Done when:** advisory evidence is publication-free and approval can reference only exact verified semantic-block evaluation.

### T012 [ ] Implement aggregate publication and pre-push

- **Class:** Implementation.
- **Depends:** T011.
- **Read first:** README sections `Candidate model`, `Evaluation and publication records`, `Owner approval`, and `Hooks and installation`; T003/T004/T008/T010/T011 handoffs; current publication parser, pre-push hook, and fixtures.
- **Settled constraints:** one push/one verdict; at most one non-delete destination tip; force/new supported; deletion-only skips candidate execution; deterministic always reruns; exact approval may skip semantic rerun only; adapters contain no policy.
- **Expected touch points:** publication module/tests/fixtures, CLI dispatch, `.githooks/pre-push`, and hook forwarding tests.
- **Deliver:** `validate --publication --updates-stdin`; aggregate base/candidate lifecycle; exact approval selection; one mechanically derived attempt; final pre-push adapter.
- **Focused gate:** malformed/deletion/new/force/mixed/multi-tip/empty input; exact rejection evidence/nullability/disposition; rejected/deletion paths invoke only `/usr/bin/git rev-parse --git-common-dir`; deterministic non-bypass; runner parity and non-HEAD diagnostic; semantic retry; candidate mutation during deterministic or semantic execution; approval binding invalidations; `pass`/`approved`/`block`; hook stdin/exit forwarding.
- **Handoff:** record final local publication, approval, and pre-push contracts consumed by T013–T016.
- **Done when:** pre-push uses final v2 publication path and one invocation writes one expected attempt.
- **Stop:** approval skips deterministic checks, publication mutates evaluation disposition, or one push yields multiple verdicts.

### T013 [ ] Implement idempotent hook installation

- **Class:** Implementation.
- **Depends:** T008, T012.
- **Read first:** README section `Hooks and installation`; T008/T012 handoffs; current hook installer, tracked hooks, CLI dispatch, and hook fixtures.
- **Settled constraints:** repository-local config only; materialized `HEAD` validation; both tracked adapters must exist and be executable; linked worktrees share Git common-dir evidence but install through local repository configuration.
- **Expected touch points:** `xtask/src/hooks.rs`, CLI dispatch, hook installer tests, `.githooks/pre-commit`, and `.githooks/pre-push` only for installer-contract corrections.
- **Deliver:** idempotent `cargo xtask hooks install` setting local `core.hooksPath=.githooks` only after validating materialized HEAD and both final tracked hooks.
- **Focused gate:** fresh/idempotent/conflicting/unborn/missing/non-executable/tool-failure/linked-worktree cases and proof both installed adapters invoke final v2 commands.
- **Handoff:** record installer contract consumed by T014/T016.
- **Done when:** fresh clone can install both final hooks without manual Git config editing.
- **Stop:** installer rewrites hooks, mutates candidate source, or silently replaces conflicting configuration.

### T014 [ ] Gate final local publication stack

- **Class:** Package gate.
- **Depends:** T013.
- **Read first:** T010–T013 handoffs and cumulative diff; README semantic, evidence, approval, candidate, and hooks sections.
- **Deterministic gate:** semantic schema/judge, report, validation commands, publication, hooks, candidate, config, and quality targets; full deterministic gate; complete local publication matrix including approved retry, rejected/deletion isolation, hook install, and linked worktrees.
- **Semantic gate:** fresh blind review of rubric ownership, fan-out isolation, coherence monotonicity, canonical evidence, approval non-bypass, aggregate push semantics, and local hook authority.
- **Handoff:** record accepted T010–T013 candidate tree, deterministic results, semantic disposition, and unresolved P2 findings.
- **Done when:** final local validation stack is green and no blocker/major remains.
- **Stop:** deterministic failure can be approved, disposition can be rewritten, or local surfaces use non-v2 dispatch.

## Package D — CI, final cleanup, documentation, and closeout

### T015 [ ] Implement final CI path and remove remaining superseded code

- **Class:** Implementation.
- **Depends:** T014.
- **Read first:** README sections `CI`, `Final-state implementation`, `Removal scope`, and `Dependency reduction target`; T006/T012/T014 handoffs; workflow, CI fixtures, semantic-v1/publication legacy paths, active quality references, `xtask/Cargo.toml`, and lockfile.
- **Settled constraints:** CI independently verifies pushed revision; local approvals ignored; event before/after bind exact base/candidate; reports upload on success/failure; no branch-protection claim; no active v1 dispatch remains.
- **Expected touch points:** workflow, CI fixtures, publication CLI/dispatch/tests for `--ci-event`, semantic-v1 and other remaining retired code/config/tests/fixtures, `xtask/Cargo.toml`, lockfile.
- **Deliver:** push-triggered `validate --publication --ci-event <path>` with canonical projection or lossless malformed source; ordinary/force/new/deletion/rejected behavior; pinned tool/judge provisioning; independent report upload; deletion of semantic-v1 runtime/config, legacy rubric registry, stale publication dispatch, trusted-base/bootstrap machinery, and dead dependencies.
- **Focused gate:** workflow syntax/action-pin audit; event matrix; malformed raw evidence; deletion/rejected process isolation; no approval input; pushed SHA/tree binding; failed artifact; local equivalent; searches prove one active manifest/semantic/publication path; full deterministic gate after deletion; old/new line and dependency counts.
- **Handoff:** record final CI contract, deleted paths/dependencies, platform provisioning recipe, and remaining stale documentation references for T016.
- **Done when:** CI uses same v2 publication lifecycle, no active retired path remains, and full gate stays green.
- **Stop:** CI diverges from local publication semantics or active code/config references retired path.

### T016 [ ] Update owner-facing validation documentation

- **Class:** Implementation.
- **Depends:** T015.
- **Read first:** README complete; T012/T013/T015 handoffs; root README, development policy, testing, architecture, technology, and active quality READMEs.
- **Settled constraints:** document observed final authority only; CI cannot prevent direct push; Git—not bespoke procedure—handles rollback.
- **Expected touch points:** named owner-facing docs and this change pack; no implementation. Escalate before editing another document.
- **Deliver:** acceptance-bullet-to-test/evidence map; final hook install, staged/advisory/publication commands, reports, approval/retry, deletion/multi-tip, CI authority, semantics, and removed-validator documentation; remove stale model/provider/parent-policy/v1 claims.
- **Focused gate:** Markdown link/path/whitespace audit and searches for contradictory commands, models, branch-protection claims, compatibility paths, migration instructions, explicit rollback procedure, and deleted validators.
- **Handoff:** preserve final acceptance map with exact documentation and evidence locations.
- **Done when:** fresh owner can install and operate final validation without reading Rust source or migration history.

### T017 [ ] Run final acceptance and independent conformance review

- **Class:** Closeout/package gate.
- **Depends:** T016.
- **Read first:** all handoffs, complete README/tasks, cumulative diff, and pre-change Git tree (`HEAD` for uncommitted execution; first authorized implementation commit's first parent otherwise).
- **Deterministic gate:** complete final manifest from clean exact candidate; full workspace gate; standalone provider and Go tests; local post-push CI equivalent where credentials permit; acceptance matrix for contamination, post-process content/mode mutation, cwd, probes, force/deletion/multi-tip, semantic fan-out/coherence/correction, approval invalidations, attempt evidence, linked-worktree storage, hook install, and CI projection; searches for retired paths and duplicate registries.
- **Semantic gate:** configured axes/coherence plus at least two fresh independent blind design/task-conformance reviewers. No reviewer reads prior output. Resolve every blocker/major, rerun affected focused targets, full gate, then only materially affected semantic review.
- **Size gate:** derive old `xtask` lines/dependencies from the pre-change Git tree and compare them to final tree; verify no reusable framework, compatibility layer, migration machinery, rollback machinery, or project policy entered generic runner.
- **Handoff:** record final accepted tree, deterministic/semantic evidence, unresolved P2 findings, macOS/Linux results, and reduction metrics.
- **Done when:** T001–T016 are `[x]`; T017 criteria pass; deterministic suite passes; semantic result is determinate or owner-approved; final CI/local behavior is coherent; macOS and Linux evidence is green; obsolete code/docs are absent; reduction evidence exists. Orchestrator then marks T017 `[x]`.
- **Stop:** policy remains hardcoded, deterministic checks can be bypassed, blindness was contaminated, retired paths remain active, or status has unexplained files.

## Completion record

At completion, append following record to canonical T017 handoff. Mirror it into tracked change-pack prose only through an owner-authorized edit:

- final candidate tree and any separately authorized commit SHA;
- full deterministic report digest;
- semantic report digest and any approval digest/reason reference;
- macOS result;
- Linux result from owner-authorized CI or named recorded Linux-container equivalent;
- old/new `xtask` source and test line counts;
- old/new direct dependency counts;
- deleted module/config/fixture inventory;
- known non-blocking follow-up work.
