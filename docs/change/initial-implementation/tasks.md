# Initial Implementation Task List

**Status:** In progress — Phase 5 SQLite authority, journal, export, and V005/F005 closure complete. Plan amended 2026-07-22: remaining work executes as work packages WP1–WP6 under the [Amended execution plan](#amended-execution-plan-2026-07-22); WP1 CLI delivery substrate is complete and owner-approved for its boundary commit

Follow [execution protocol](README.md#execution-protocol), resolve [decision gates](decisions.md), and maintain [coverage map](coverage.md). Every task is sized as one narrow orchestrator-owned unit unless its stop condition requires owner escalation.

File contracts resolve deterministically from repository root. Paths beginning `../../` or `../../../` are explicitly relative to this task file's directory; bare local pack names (`README.md`, `decisions.md`, `tasks.md`, `coverage.md`) mean `docs/change/initial-implementation/`; all other paths/globs are repository-root-relative. `tests` beside a primary Rust path means inline `#[cfg(test)]` module in that same file unless an exact test path is named. Orchestrator alone updates task markers/range ledger in `tasks.md`; that administrative edit is permitted at every handoff but grants no other file.

Path aliases used below are exact:

- `core/` = `crates/loop-engine-core/`
- `integrations/` = `crates/loop-engine-integrations/`
- `cli/` = `crates/loop-engine-cli/`.

Phase 8 shorthand is also exact:

- `provider command` = `cli/src/commands/provider.rs`; `provider command/catalogs` adds `cli/src/args.rs`, `core/src/operations/catalog.rs`, `cli/src/driver_catalog.rs`;
- `run command` = `cli/src/commands/run.rs`; `run command/catalogs` adds `cli/src/args.rs` and both catalog files above;
- `provider/run command catalogs` = both command files, `cli/src/args.rs`, and both catalog files above;
- `evidence command` = `cli/src/commands/evidence.rs`; `evidence command/catalogs` adds `cli/src/args.rs` and both catalog files above;
- `export command` = `cli/src/commands/export.rs`; `export command/catalogs` adds `cli/src/args.rs` and both catalog files above;
- bare E2E module `<name>.rs` = `cli/tests/e2e/<name>.rs` under `cli/tests/e2e.rs`;
- `docs/coverage` = `docs/operation-catalog.md`, `docs/cli-contract.md`, this `coverage.md`, plus task-named affected foundation/protocol docs; `tasks.md` markers remain orchestrator-only;
- each exposure always stages its exact `quality/facets/v1/<operation-id>.json` path named by task.

Validation command conventions — these commands are executed by work-package validation rituals and orchestrator boundary rituals (historically by junction V-tasks through V005), never by individual task owners:

- focused core test: `cargo test -p loop-engine-core <module-filter>`;
- focused integration test: `cargo test -p loop-engine-integrations <module-filter>`;
- targeted CLI E2E: `cargo test -p loop-engine-cli --test e2e <task-listed-filter>`;
- targeted reference E2E: `cargo test -p loop-engine-cli --test reference <task-listed-filter>`;
- operation closure: `cargo run -p xtask -- operation-coverage`;
- architecture: `cargo run -p xtask -- architecture`;
- documentation: `cargo run -p xtask -- docs-check`;
- uncommitted junction full gate: `cargo run -p xtask -- quality --root .`; post-commit exact-revision confirmation: `cargo run -p xtask -- quality --revision <commit-sha>`.

A task saying “targeted E2E” means exact CLI E2E command above with operation/scenario name as filter. A task saying “targeted reference E2E” means exact reference command above with behavior-group module as filter. `cli/tests/e2e/*.rs` are modules of `cli/tests/e2e.rs`; `cli/tests/reference/*.rs` are modules of `cli/tests/reference.rs`. For focused core/integration tests, module filter is exact basename of task's primary Rust module (for example T043 `gate`, T105 `migrate`). Closure has four explicit stages: `baseline` requires all runtime sets empty; `candidate --allow-open <comma-separated-stable-ids>` requires core/driver/route equality for named uncommitted atomic-group IDs while permitting their declared open facets; default `exposed` requires exact core=driver=route=E2E=trace equality and closed manifest for every currently exposed ID; `final` adds exact D004 21-ID equality. Bare “closure” means `exposed`; `candidate` appears only inside WP3 checkpoint staging; `final` applies only at WP6 change close. These conventions define the accumulated deterministic check inventory: `git diff --check`; format/Clippy (T020+); `cargo check --workspace --locked` (T017+); `cargo test --workspace` (first tests+); architecture (T021+); docs check (T026+); dependency policy (T030+); `cargo doc --workspace --no-deps` (T052+); operation closure in the current stage mode (T062+); standalone fixture-crate tests via `cargo test --manifest-path test-support/providers/<crate>/Cargo.toml --locked` (T135/T140+); full CLI E2E suite (T143+); reference suite (T185+); quality manifest gate (T028+). A work-package validation ritual runs every inventory command whose implementing work is complete, plus its package-targeted suites.

Task bullet vocabulary: `**Tests:**` names test/fixture content the task must author as deliverables; `**Criteria:**` names consistency requirements its artifacts must satisfy. Neither bullet asks for per-task verification-command execution. The orchestrator implements T-tasks directly, then executes accumulated validation at junction V-tasks, batches valid repairs under F-tasks, and performs the boundary ritual.

## Junction governance tasks and owner cadence

> **Amendment (2026-07-22):** junctions `V001`–`V005`/`F001`–`F005` executed and closed; the material below is their historical record and remains authoritative for completed ranges. Junctions `V006`–`V013`/`F006`–`F013` are retired unexecuted; all remaining validation and repair follows [Work-package governance](#work-package-governance) in the amended execution plan.

Junction validation tasks `V001`–`V013` and junction fix tasks `F001`–`F013` sit outside T numbering. Owner-directed corrective tasks `R001`–`R005` repair the C1 governance implementation after the publication-checkpoint policy superseded per-commit scheduling. These are net-new governance tasks; implementation task count remains exactly T001–T200 and no T-task is renumbered. Four execution modes are distinguished:

| Mode | Owner | Scope |
|---|---|---|
| Task implementation | orchestrator directly | author code/tests/fixtures/docs per T-task contract; defer accumulated command execution to next V-task |
| Junction validation | orchestrator directly, one V-task per junction | read-only execution of accumulated deterministic inventory plus range-targeted suites; evidence may be written under `target/junction-evidence/V0xx/` |
| Batched validation repair | orchestrator directly, one F-task per junction | batch every valid finding from paired V-task; explicit no-op completion when validation was clean |
| Blind correctness review | periodic Fable + GPT-Sol blind subagents, followed by orchestrator repair | independent correctness/completeness judgment; only remaining subagent role; entirely separate from V/F tasks |

V and F tasks are deterministic validation/repair rounds only — they are never a Fable/Sol correctness review round and produce no P0–P3 findings under the README review contract.

Junction placement and gating:

| Junction | Validates range | Gated boundary commit | First gated task |
|---|---|---|---|
| V001/F001 | T017–T030 | T030 `build: establish workspace governance` | T031 |
| V002/F002 | T031–T052 | T052 `feat(core): establish domain model` | T053 |
| V003/F003 | T053–T083 | T083 `feat(core): add private operations` | T084 |
| V004/F004 | T084–T103 | T103 `feat(integrations): add provider config and trace substrate` | T104 |
| V005/F005 | T104–T119 | T119 `feat(persistence): add transactional state and journal` | T120 |
| V006/F006 | T120–T134 | T134 `feat(cli): add private driver and contracts` | T135 |
| V007/F007 | T135–T145 | T145 `test(e2e): establish black-box harness` | T146 |
| V008/F008 | T146–T152 | T152 `feat(run): expose provider check and run foundation` | T153 |
| V009/F009 | T153–T158 | T158 `feat(provider): expose provider.restore` | T159 |
| V010/F010 | T159–T166 | T166 `feat(run): expose run.export` | T167 |
| V011/F011 | T167–T184 | T184 `test(e2e): close cross-operation acceptance` | T185 |
| V012/F012 | T185–T190 | T190 `test(reference): publish reference acceptance report` | T191 |
| V013/F013 | T191–T194 | T194 `docs: finalize operations and recovery` | T195 |

Junction rules:

- a junction opens when the last T-task of its range reports implementation deliverables complete; this does not mean range-verified. Junction closes only when its V-task reports clean and its F-task is complete (no-op or repaired);
- the gated boundary commit and every later T-task wait for junction closure; no task whose ID follows a junction's range may enter `[~]` while that junction is open, even where its `Depends` line does not name the junction explicitly (covers cross-phase dependencies such as T054, T057, T062);
- V-tasks are read-only except generated untracked evidence and never edit tracked files, task markers, or git state;
- F-tasks batch all valid V findings into one pass; a non-no-op F-task requires targeted revalidation — the orchestrator re-invokes the same V-task in rerun mode (failed checks first, then the full accumulated inventory) before the junction closes; a no-op F-task requires no rerun;
- for ranges containing earlier per-task commits (V008, V009, V010, V011, V012), the junction validates the cumulative candidate tree before the final gated boundary commit; F-task repair paths append to the gated boundary's range ledger;
- V/F execution never commits independently; orchestrator updates markers and ledgers, then commits only under README boundary ritual;
- orchestrator may explicitly re-run any V-task out of cadence; results follow same rules.

V-task contract template (applies to every V-task; per-task blocks list only range and additions): depends on its range's final T-task; runs every accumulated-inventory command whose implementing task is complete plus the range's targeted suites and stage-appropriate closure mode; records each failure as a finding (file/path, failing command, exact output excerpt) in `target/junction-evidence/V0xx/findings.md`; files a clean report when everything passes; stops for orchestrator. F-task contract template: depends on its paired V-task; applies one batched repair pass across the validated range's files for all valid findings, or records explicit no-op; appends changed paths to the gated boundary's range ledger; stops for orchestrator.

Before T026 exists, Phase 0 documentation verification means `git diff --check` plus this exact repository-independent check:

```bash
python3 - <<'PY'
from pathlib import Path
import re
for p in Path('docs').rglob('*.md'):
    text = p.read_text(encoding='utf-8')
    assert text.endswith('\n'), p
    assert not re.search(r'[ \t]+$', text, re.M), p
    for target in re.findall(r'\[[^]]+\]\(([^)#]+)', text):
        if '://' not in target:
            assert (p.parent / target).resolve().exists(), (p, target)
PY
```

Global rules for every task:

- no OpenSpec artifacts or commands;
- no unrelated edits;
- no mock frameworks or mock-based behavioral tests;
- external DTO annotations never enter core model;
- every changed public contract receives documentation in the same publication checkpoint;
- run `git diff --check` before handoff;
- no T/V/F step commits independently; every authorized boundary uses orchestrator README ritual;
- per-task implementation steps execute no build/test/lint/closure commands; work-package validation rituals own accumulated validation and repair (junction V/F steps owned this through V005);
- WP3 checkpoint groups are indivisible same-owner assignments;
- public operation enters runtime catalog only in its exposure task.

## Phase 0 — Freeze implementation contracts

### T001 [x] Resolve runtime packaging and persistence direction
- **Depends:** none
- **Read:** `decisions.md` D001; foundation technology, architecture, invariants C1–C4.
- **Files:** `decisions.md`, `../../invariants.md`, `../../technology.md`, `../../architecture.md`, `../../testing.md`.
- **Deliver:** owner-approved native CLI/SQLite/gate/`xtask`/version/distribution policy plus exact crate-by-crate dependency/version/feature table for core, integrations, CLI, xtask, and standalone provider fixtures; root workspace explicitly excludes fixture crates; update technology statuses and resolution record.
- **Done when:** no dependent task must treat C1–C4 as candidate.
- **Stop:** owner rejects recommendation; re-plan every persistence/tooling task.

### T002 [x] Resolve supported platforms
- **Depends:** T001
- **Read:** D002; foundation trace, subprocess, distribution, and test isolation sections.
- **Files:** `decisions.md`, `../../technology.md`, `../../testing.md`.
- **Deliver:** exact MVP OS/architecture matrix and unsupported-platform policy published in `technology.md` and mirrored only where testing scope differs.
- **Done when:** process, permission, path, fixture, and CI tasks share one platform scope.
- **Stop:** Windows included; reopen D005, D007, D010, and D013 first.

### T003 [x] Select project license
- **Depends:** T001
- **Read:** D003 and dependency/release requirements.
- **Files:** `decisions.md`, `../../technology.md`, new `../../../LICENSE-MIT`, new `../../../LICENSE-APACHE`, new `../../../README.md` as selected by D003.
- **Deliver:** owner-selected canonical license file(s), root README license notice, and resolution record; T017 consumes decision into exact package metadata.
- **Done when:** dependency allowlist and package metadata can use one explicit policy.
- **Stop:** owner has not selected MIT or MIT/Apache-2.0.

### T004 [x] Freeze application operation catalog
- **Depends:** T001
- **Read:** D004; architecture operation list; all UX storyboards; testing closed-catalog rule.
- **Files:** `decisions.md`, new `../../operation-catalog.md`, `quality/facets/v1/{schema.json,README.md}`, `../../architecture.md`, `../../testing.md`, `../../ux-storyboards.md`.
- **Deliver:** stable IDs, exact command/argv/flag ownership, exact facet flags, D005 role/result-derived provider rows including every applicable evaluation error, stable reason-code taxonomy, facet-inventory v1 schema/path/status/evidence rules, normative lifecycle-family owner table matching `testing.md`, registration-wide `provider.check --active-runs` versus per-run compatibility ownership, update-without-approval versus disable-acknowledgement semantics, exact 21-ID architecture list, `run compatibility` storyboard example, mutation classification, explicit non-operations, and rationale for update/compatibility/export.
- **Criteria:** every desired caller action and foundation outcome maps once; provider catalog success/rejection requires fresh catalog proof and no run journal; rejected creation has no run/journal; post-lookup run rejections require fresh history.
- **Done when:** exact catalog accepted and coverage table can be mechanical.
- **Stop:** any operation split/merge remains disputed.

### T005 [x] Freeze provider protocol v1 transport
- **Depends:** T001, T002
- **Read:** D005; provider model in intent/architecture/technology; I4, I6, I8, I9, I20, I32, I37–I40, I43–I44.
- **Files:** `decisions.md`, new `../../provider-protocol-v1.md`, `../../technology.md`, `../../architecture.md`.
- **Deliver:** process lifetime, stdin/stdout framing, version negotiation, five role names, role-specific result applicability matrix, inherited-environment/no-override/no-trace policy, resolve-once immutable config handoff, unknown-field policy, timeout/termination, and no-state-authority rule.
- **Criteria:** protocol examples are language-neutral; each role maps only valid denial/finding/incompatibility/error variants; invoker cannot query catalog.
- **Done when:** independent provider author could implement transport without Rust source.
- **Stop:** persistent process, streaming protocol, or Windows support requested; re-design before schemas.

### T006 [x] Freeze structured CLI and exit contract
- **Depends:** T004
- **Read:** D006; I18, I27, I34, I46; UX automation storyboard.
- **Files:** `decisions.md`, new `../../cli-contract.md`, `../../ux-storyboards.md`, `../../testing.md`.
- **Deliver:** schema version, additive versus breaking compatibility rule/support duration, exact argv/flag surface, top-level fields, run summary, evidence status, reason format, stdout/stderr boundary, and exit codes.
- **Criteria:** completed/rejected/error plus pre-dispatch examples validate manually against contract.
- **Done when:** renderer and E2E parser require no schema guesses.
- **Stop:** human and structured semantics diverge.

### T007 [x] Freeze machine-local paths and configuration
- **Depends:** T001, T002
- **Read:** D007; intent configuration; I16, I40–I41; technology persistence direction.
- **Files:** `decisions.md`, new `../../configuration.md`, `../../technology.md`, `../../architecture.md`.
- **Deliver:** exact config/state/trace paths, `LOOP_ENGINE_HOME`, TOML shape, precedence, unknown-key policy, symlink/lexical normalization, nonexistent executable/CWD behavior, configured-spelling identity rule, absolute registration paths, and malformed-config behavior.
- **Criteria:** examples cover normal user home, isolated test home, symlink/nonexistent paths, unknown keys, and another caller CWD.
- **Done when:** project defaults cannot rebind an existing run.
- **Stop:** multiple state stores or provider definitions in project config proposed.

### T008 [x] Freeze bounds and timeout defaults
- **Depends:** T005, T006, T007
- **Read:** D008; evidence, provider, trace, and diagnostic bounds in foundation.
- **Files:** `decisions.md`, `../../provider-protocol-v1.md`, `../../configuration.md`, `../../technology.md`, `../../cli-contract.md`.
- **Deliver:** one named table for every payload/scalar/label/note/actor/path/argv/config/diagnostic/trace/timeout bound; count+byte keyset pagination for registration/run/evidence/history/active-compatibility/impact reports; ten-call provider-page ceiling and cross-process trace reservations; default/max count and page-byte budget; empty/malformed/wrong-version/filter-mismatched cursor behavior; CLI/schema owners.
- **Criteria:** each bound/pagination rule has one source of truth and cursor-v1 schema/examples live in `cli-contract.md`; E2E owners are T147 registration/zero-impact list, T150/T152 run/history and active-compatibility pages, T157/T175 nonempty impact pages, T160 evidence list, and T101/T152/T182 trace-reservation limits.
- **Done when:** no implementation task needs a magic number.
- **Stop:** selected evidence would be truncated; foundation requires rejection instead.

### T009 [x] Freeze SQLite and migration policy
- **Depends:** T001, T007
- **Read:** D009; persistence authority; I12–I16, I35–I36, I45.
- **Files:** `decisions.md`, new `../../persistence.md`, `../../technology.md`, `../../architecture.md`, `../../testing.md`.
- **Deliver:** pragmas, busy policy, migration rules, schema-version compatibility, transaction boundaries, workflow and registration-config CAS semantics, atomic affected-run digest guard for catalog mutation, create-versus-update/disable/restore linearization, and no-write-lock provider rule.
- **Criteria:** stale event/create attempts, both catalog/create writer orders, label/note/evidence concurrency, and rollback narratives are explicit.
- **Done when:** migration `0001` can be designed without semantic gaps.
- **Stop:** event sourcing, ORM, dual authority, or provider-spanning write transaction proposed.

### T010 [x] Freeze operational trace contract
- **Depends:** T002, T006–T008
- **Read:** D010; I42, I46; architecture/technology trace sections; UX trace contract.
- **Files:** `decisions.md`, new `../../operational-trace.md`, `../../technology.md`, `../../testing.md`, `../../ux-storyboards.md`.
- **Deliver:** JSONL v1 categories, permissions/startup/flush, encoded-byte actual-plus-reserved hard rotation budget, no raw/parsed duplication, concurrent coordination, help/version/parse behavior, late-sink semantics, and external `SIGXFSZ`/`RLIMIT_FSIZE` read-plus-mutation E2E contract.
- **Criteria:** completed/rejected/error/parse/init/crash examples plus provider-read and committed-annotation `EFBIG` cases on supported Unix without production test branch.
- **Done when:** trace cannot become competing state/journal authority.
- **Stop:** design requires trace context in every function or claims impossible crash completeness.

### T011 [x] Freeze journal entry model
- **Depends:** T008–T009
- **Read:** D011; I8, I11–I15, I35; reference journal expectations.
- **Files:** `decisions.md`, new `../../journal-contract.md`, `../../architecture.md`, `../../ux-storyboards.md`.
- **Deliver:** entry kinds, 2.5 MiB aggregate/component bounds, attempt shape, required fields, sequence, correction link, provider/gate/evidence nesting, and non-replay statement.
- **Criteria:** creation/mutation/rejection/provider/stale/guidance/correction examples plus maximum-size arithmetic and oversize rejection.
- **Done when:** persistence schema and history renderer share one immutable model.
- **Stop:** proposed model requires reconstructing current state by replay.

### T012 [x] Freeze semantic judge contract and provisioning
- **Depends:** T001
- **Read:** D012; I47; testing Git enforcement; architecture composition/enforcement.
- **Files:** `decisions.md`, new `../../development-policy.md`, `../../testing.md`, `../../architecture.md`.
- **Deliver:** generic executable request/result v1, foundation-seed rubric manifest for parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`, no-second-bootstrap rule, local/publication outcomes, timeout, citations, deterministic-evidence input, actual local judge executable/configuration, and named CI secret/config owner.
- **Criteria:** real local publication-mode smoke against foundation parent plus pass/fail/indeterminate/unavailable fixture matrix; first post-foundation push remains blocked until T029.
- **Done when:** owner proves real judge runs locally and records exact T029 provisioning handoff without storing credentials.
- **Stop:** fixture/fake judge is proposed as publication authority.

### T013 [x] Freeze provider-fixture implementation strategy
- **Depends:** T002, T005
- **Read:** D013; no-mock doctrine; reference provider boundary.
- **Files:** `decisions.md`, `../../testing.md`, `../../technology.md`.
- **Deliver:** fixture language/runtime, package location, subprocess isolation, invocation ledger, and process-failure helper policy.
- **Criteria:** fixture remains external executable and software concepts remain outside core.
- **Done when:** CI/runtime prerequisites are known.
- **Stop:** fixture imports product internals or accesses authoritative DB.

### T014 [x] Freeze canonical graph encoding
- **Depends:** T005, T008
- **Read:** D014; I6–I8, I32, I37, I43; graph validation direction.
- **Files:** `decisions.md`, `../../provider-protocol-v1.md`, new `../../graph-projection.md`, `../../technology.md`.
- **Deliver:** semantic ordering rules, included fields, exact canonical DTO version, metadata treatment, golden vectors, and SHA-256 identity distinction.
- **Criteria:** field-change matrix states whether graph revision must change.
- **Done when:** no persisted run can later receive accidental digest redefinition.
- **Stop:** canonical form depends on hash-map iteration or provider raw JSON formatting.

### T015 [x] Freeze audit export scope
- **Depends:** T004, T006, T009, T011
- **Read:** D015; I24, I41, I45; persistence/export testing requirements.
- **Files:** `decisions.md`, new `../../export-contract.md`, `../../operation-catalog.md`, `../../testing.md`, `../../technology.md`.
- **Deliver:** `run.export` ownership, output directory behavior, state JSON/journal JSONL schemas, ordering, D006 additive/breaking compatibility and support-duration policy, and no-import guarantee.
- **Criteria:** export cannot mutate, restore, replay, or dereference locators.
- **Done when:** optional foundation language and required test language agree.
- **Stop:** export to stdout would violate one-envelope structured mode without explicit design.

### T016 [x] Freeze project-default discovery and close C0
- **Depends:** T001–T015
- **Read:** D016; I40–I41; clean-room/no-discovery constraints.
- **Files:** `decisions.md`, `../../configuration.md`, `../../technology.md`.
- **Deliver:** ancestor-search algorithm, file name, boundary, allowed keys, precedence, and explicit distinction from provider discovery.
- **Criteria:** examples prove no registration/executable rebinding.
- **Done when:** all D001–D016 rows and foundation updates pass; stop for orchestrator C0 commit `docs: freeze implementation contracts` with complete Phase 0 staged scope.
- **Stop:** project file becomes workflow/provider authoring source.

## Phase 1 — Workspace, governance, and architecture skeleton

### T017 [x] Create reproducible empty Rust workspace shell
- **Depends:** T001–T016
- **Read:** I22–I24; architecture product crates; selected license/toolchain decisions.
- **Files:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.gitignore`.
- **Deliver:** Rust 2024 virtual workspace with empty member list, explicit `test-support/providers/*` exclusion, shared package version `0.1.0`, exact T001 toolchain/resolver, and no MSRV/installer claim; T018/T019 add members.
- **Done when:** workspace shell parses from clean checkout without naming nonexistent members.
- **Stop:** any fourth product/runtime crate is introduced.

### T018 [x] Create three product crate roots
- **Depends:** T017
- **Files:** `Cargo.toml`, `Cargo.lock`, `crates/loop-engine-core/{Cargo.toml,src/lib.rs}`, `crates/loop-engine-integrations/{Cargo.toml,src/lib.rs}`, `crates/loop-engine-cli/{Cargo.toml,src/main.rs}`.
- **Deliver:** add exactly three product members; preload complete T001-approved runtime/dev dependency versions/features into each manifest and lockfile so later product tasks need no manifest invention; add compile-only roots and inward dependency direction.
- **Done when:** CLI alone depends on both product libraries and is composition root placeholder.
- **Stop:** core receives Clap, Serde, SQLite, process, config, or tracing-subscriber dependency.

### T019 [x] Create `xtask` command shell
- **Depends:** T017
- **Files:** `Cargo.toml`, `Cargo.lock`, `xtask/{Cargo.toml,src/main.rs,src/lib.rs}`.
- **Deliver:** add non-shipping workspace member with complete T001-approved xtask dependencies/lock update and command dispatcher with only implemented subcommands.
- **Done when:** unknown command fails clearly and no product crate depends on `xtask`.
- **Stop:** gate logic duplicated in shell scripts.

### T020 [x] Configure formatting, lint, and warnings policy
- **Depends:** T017–T019
- **Files:** `Cargo.toml`, `rustfmt.toml`, `../../development-policy.md`.
- **Deliver:** workspace formatting and Clippy warnings-denied commands with no broad allow list.
- **Done when:** canonical commands are documented once.
- **Stop:** lint suppression hides unfinished behavior.

### T021 [x] Implement crate dependency architecture check
- **Depends:** T018–T019
- **Read:** architecture dependency direction; I22–I24.
- **Files:** `xtask/src/architecture.rs`, `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/{allowed,forbidden-edge}/*`.
- **Deliver:** Cargo-metadata check for exactly three product crates and allowed dependency edges.
- **Tests:** deliberate forbidden-edge fixture fails.
- **Done when:** core outer dependency and integrations→CLI edge are mechanically rejected.
- **Stop:** check relies only on prose or current successful build.

### T022 [x] Implement internal-boundary and bypass architecture checks
- **Depends:** T021
- **Read:** architecture model/capabilities/operations direction and choke-point rules.
- **Files:** `xtask/src/architecture.rs`, `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/{bypass,reversed-core,catch-all}/*`.
- **Deliver:** checks for model←capabilities←operations direction, sole composition root, approved provider/persistence/dispatch modules, and forbidden `util`/`common` catch-alls.
- **Tests:** canaries for direct process/SQLite construction and reversed core import fail.
- **Done when:** check enforces dependency/bypass boundaries without counting log syntax.
- **Stop:** custom compiler plugin or broad AST framework proposed.

### T023 [x] Define semantic-judge v1 schemas
- **Depends:** T012, T019
- **Files:** `quality/semantic-judge/v1/{request,response}.schema.json`, `quality/semantic-judge/v1/fixtures/*.json`, `xtask/tests/semantic_judge_schema.rs`.
- **Deliver:** generic request and pass/fail/indeterminate response schemas with citations and deterministic evidence references.
- **Tests:** valid/malformed request/response fixtures.
- **Done when:** schemas contain no Pi/model/provider-specific field.
- **Stop:** response permits uncited pass/fail or candidate rubric substitution.

### T024 [x] Implement semantic-judge runner
- **Depends:** T019, T023
- **Files:** `xtask/src/semantic_judge.rs`, `xtask/tests/semantic_judge.rs`.
- **Deliver:** exact base/candidate diff collection through generic parent/candidate protocol fields, base rubric loading, executable invocation, timeout, schema validation, local/publication disposition.
- **Tests:** tests for pass, fail, indeterminate, unavailable, timeout, malformed response, foundation seed-rubric fallback, aggregate unpublished range, and changed-rubric-next-push behavior.
- **Done when:** publication mode fails closed and product runtime has no judge dependency.
- **Stop:** runner silently falls back to pass.

### T025 [x] Author focused parent-versioned rubrics
- **Depends:** T012, T024
- **Files:** `quality/rubrics/{documentation,observability,architecture,behavioral-evidence}.md`, `quality/rubrics/manifest.json`, `quality/rubrics/fixtures/*.json`, `xtask/tests/semantic_judge.rs`.
- **Deliver:** narrow cited criteria matching foundation, including KISS and no invented test claims.
- **Tests:** manifest version/hash and cited example-decision fixtures.
- **Done when:** parent revision can supply complete rubric set deterministically.
- **Stop:** rubric changes foundation policy without owner decision.

### T026 [x] Implement deterministic documentation checker
- **Depends:** T019
- **Files:** `xtask/src/docs_check.rs`, `xtask/tests/docs_check.rs`, `xtask/tests/fixtures/docs-check/*`.
- **Deliver:** UTF-8, final newline, trailing whitespace, relative-link, duplicate-heading, required-file, and terminology checks.
- **Tests:** focused invalid fixtures.
- **Done when:** semantic judge is not asked to replace deterministic checks.
- **Stop:** checker attempts subjective semantic judgment.

### T027 [x] Implement hook installer and pre-commit adapter *(historical implementation; scheduling superseded by R002)*
- **Depends:** T024–T026
- **Files:** `.githooks/pre-commit`, `xtask/src/hooks.rs`, `xtask/tests/hooks.rs`, `xtask/tests/fixtures/hooks/*`.
- **Deliver:** versioned thin hook, exact staged-tree materialization, install/verify commands, and original local semantic attempt.
- **Tests:** staged pass, semantic fail, unavailable warning, unstaged contamination, and hook-version mismatch tests.
- **Done when:** historical C1 implementation is present; R002 must remove expensive quality and semantic execution from default pre-commit without weakening exact-index purity.
- **Stop:** hook judges working tree instead of exact staged tree.

### T028 [x] Implement exact-commit pre-push gate and incremental quality manifest *(historical implementation; scheduling superseded by R003)*
- **Depends:** T024–T027
- **Files:** `.githooks/pre-push`, `xtask/src/{publication,quality}.rs`, `quality/manifest.toml`, `xtask/tests/{publication,quality}.rs`, `xtask/tests/fixtures/publication/*`.
- **Deliver:** original per-commit publication engine and incremental manifest foundation retained as historical C1 work.
- **Tests:** original exact-commit fixtures remain until R003 replaces their expected behavior.
- **Done when:** historical implementation is present; R003 must replace enumeration with one exact remote-base-to-candidate-head gate.
- **Stop:** aggregate gate trusts stale remote-tracking refs or evaluates any internal commit separately.

### T029 [x] Add authoritative CI adapter *(historical implementation; scheduling superseded by R004)*
- **Depends:** T028
- **Files:** `.github/workflows/quality.yml`, `../../development-policy.md`.
- **Deliver:** original CI provisioning and canonical-command adapter retained as historical C1 work.
- **Criteria:** local dry-run command documentation.
- **Done when:** historical implementation is present; R004 must run one aggregate semantic job and candidate-head deterministic platform jobs without YAML gate duplication.
- **Stop:** real judge cannot be provisioned; publication remains blocked.

### T030 [x] Add dependency policy and close C1
- **Depends:** T003, T017–T029
- **Files:** `deny.toml`, `xtask/src/dependencies.rs`, `xtask/tests/dependencies.rs`, `../../development-policy.md`, `quality/evidence/v1/checkpoint.schema.json`.
- **Deliver:** allowed licenses/sources, advisory policy, lockfile checks, canonical command, and C1 closure covering workspace/governance/empty runtime surface.
- **Tests:** dependency-policy fixture violations fail.
- **Done when:** C1 candidate is buildable, governed, architecture/dependency-policy-clean, exposes zero application operations, and stops for orchestrator commit `build: establish workspace governance`.
- **Stop:** required dependency conflicts with owner-selected license.

### V001 [x] Junction validation — governance skeleton (T017–T030)
- **Depends:** T030
- **Owner:** orchestrator in V-mode; per junction template above; not a Fable/Sol review round.
- **Files:** none tracked; untracked `target/junction-evidence/V001/**` only.
- **Deliver:** execute every accumulated-inventory command implemented by T030 (hygiene, format, Clippy, locked check, workspace tests, architecture, docs check, dependency policy) against the C1 candidate; findings ledger or clean report.
- **Done when:** every command has a recorded outcome, findings are handed to F001 or a clean report is filed, and no tracked file changed; stop for orchestrator.

### F001 [x] Junction fix — batch V001 findings
- **Depends:** V001
- **Owner:** orchestrator in F-mode; one batched pass; per junction template above; not a correctness review round.
- **Files:** files within T017–T030 contracts as valid findings require; changed paths append to the T030 range ledger.
- **Deliver:** one batched repair pass for all valid V001 findings, or explicit no-op completion when V001 is clean.
- **Range ledger:** [`c1-range-ledger.txt`](c1-range-ledger.txt) is the exact sorted union of T017–T030 implementation and F001 repair paths; V001 evidence remains ignored under `target/`.
- **Done when:** no-op is recorded, or repairs are complete and targeted V001 rerun reports clean; the T030 boundary commit and T031 stay blocked until junction closure; stop for orchestrator.

## Owner-directed governance repair — aggregate publication checkpoints

### R001 [x] Replace independent-commit coherence contracts
- **Depends:** owner veto after T030
- **Files:** `../../{intent,tenets,invariants,architecture,testing,development-policy}.md`, `README.md`, `decisions.md`, `tasks.md`, `coverage.md`, `../../../quality/{rubrics,semantic-judge/v1}/**` as policy wording requires.
- **Deliver:** publication-checkpoint coherence, exact remote-base-to-candidate-head authority, intermediate-commit repair allowance, base-rubric-next-push semantics, explicit supersession of per-commit scheduling, and one owner-selected aggregate migration rubric for the exact foundation-base transition.
- **Done when:** governing docs and task graph contain no current per-commit publication requirement; immutable historical seed artifacts remain identified as historical.

### R002 [x] Make default pre-commit bounded and deterministic
- **Depends:** R001, T027
- **Files:** `.githooks/pre-commit`, `xtask/src/hooks.rs`, `xtask/tests/hooks.rs`, hook fixtures as needed.
- **Deliver:** exact staged-tree `git diff --check`, docs, architecture, and formatting only; retain contamination/toolchain/Git isolation; keep staged semantic judge as explicit command.
- **Tests:** fast staged pass/failure, no semantic invocation, no full manifest/test/Clippy/deny invocation, contamination, and hook-version mismatch.
- **Done when:** normal commit does not invoke model/network or full quality suite.

### R003 [x] Replace per-commit publication with one aggregate range gate
- **Depends:** R001–R002, T028
- **Files:** `.githooks/pre-push`, `xtask/src/{publication,quality,hooks}.rs`, `xtask/tests/{publication,quality,hooks}.rs`, publication fixtures as needed.
- **Deliver:** one exact advertised remote-base-to-candidate-head diff, one detached head worktree, one candidate-head manifest quality run, one base-rubric semantic request/response, fail-closed disposition, exact new-branch integration-base resolution, rejection of ambiguous multi-ref pushes, and candidate-bound gate execution.
- **Tests:** multi-commit range emits one result; internal bad-then-repaired history may pass final range; bad final head blocks; base advance/rebase invalidates pair; stale/forged tracking refs cannot select base; manifest weakening from base blocks.
- **Done when:** no publication path iterates internal commits for quality or semantic judgment.

### R004 [x] Split aggregate semantic CI from candidate-head platform quality
- **Depends:** R003, T029
- **Files:** `.github/workflows/quality.yml`, `../../development-policy.md`, workflow-facing publication tests.
- **Deliver:** protected-default-branch `pull_request_target` workflow with one credential-free trusted-base quality-evidence job, one fresh-runner `publication / aggregate` semantic authority job, four credential-free deterministic `quality / ...` candidate-head jobs, one base/head-bound response artifact, and exact required-check handoff.
- **Done when:** CI never invokes semantic judge per platform or per internal commit, candidate code never executes with judge credentials, and server authority remains fail-closed.

### R005 [x] Validate and authorize governance repair checkpoint
- **Depends:** R001–R004
- **Files:** tracked repairs only when validation finds defects; untracked evidence under `target/junction-evidence/R005/**`.
- **Deliver:** focused hook/publication/quality/schema tests, full current manifest, semantic contract verification, blind Fable/GPT-Sol correctness reviews, and a finalized foundation-to-HEAD publication candidate.
- **Done when:** all local checks pass, blind reviews report no P0/P1, and the owner authorizes the finalized candidate for one aggregate push attempt; remote acceptance is an external release gate and intentionally requires no post-push marker commit.
- **Publication gate:** each push attempt performs exactly one aggregate foundation-to-candidate judgment; T031 remains blocked until the remote branch equals the authorized checkpoint.
- **Stop:** any path still requires per-internal-commit execution or authoritative aggregate judge is unavailable/indeterminate.

## Phase 2 — Core model and deterministic semantics

### T031 [x] Establish core module boundaries
- **Depends:** T030, V001, F001, R005
- **Files:** `crates/loop-engine-core/src/{lib.rs,model/mod.rs,capabilities/mod.rs,operations/mod.rs}`.
- **Deliver:** private-by-default model/capability/operation layout with documented dependency direction.
- **Done when:** no empty generic service/repository/util abstraction exists.
- **Stop:** an outer DTO is needed in core API; redesign boundary first.

### T032 [x] Implement bounded scalar primitives
- **Depends:** T008, T031
- **Files:** `core/src/model/bounded.rs` (inline tests).
- **Deliver:** bounded text, diagnostic, metadata depth/count, and encoded-size validation without serialization annotations.
- **Tests:** boundary, Unicode-byte, depth, and oversize tests.
- **Done when:** all later model types reuse named bounds.
- **Stop:** truncation is used where foundation requires rejection.

### T033 [x] Implement validated identifiers
- **Depends:** T008, T031–T032
- **Files:** `core/src/model/ids.rs`, tests.
- **Deliver:** distinct registration/run/request/state/event/gate/evidence/journal/graph-revision IDs and exact D004 lowercase-ASCII/no-normalization provider handle grammar.
- **Tests:** valid/empty/oversize/cross-type plus explicit 1-, 2-, 3-, and 128-character handle vectors, uppercase, Unicode, and invalid-character tests.
- **Done when:** IDs are not interchangeable accidentally and carry no external format derives.
- **Stop:** labels or executable paths become identity.

### T034 [x] Implement time and internal version facts
- **Depends:** T031
- **Files:** `core/src/model/time.rs`, `version.rs`, tests.
- **Deliver:** timestamp value, workflow-state/lifecycle version, and journal sequence types.
- **Tests:** monotonic version/sequence transition tests.
- **Done when:** caller cannot construct or supply workflow revision token through public operations.
- **Stop:** label/note/evidence versions are coupled to gate CAS version.

### T035 [x] Implement actor metadata and note model
- **Depends:** T032–T033
- **Files:** `core/src/model/annotation.rs`, tests.
- **Deliver:** bounded opaque actor metadata, notes, and correction link without authority methods.
- **Criteria:** same decision inputs with different actor metadata remain equal in resolver tests later.
- **Done when:** no human/agent enum exists.
- **Stop:** actor field can satisfy gate or authorize transition.

### T036 [x] Implement immutable input declarations and values
- **Depends:** T032–T033, T005, T008
- **Files:** `core/src/model/run_input.rs`, tests.
- **Deliver:** provider-declared required/optional names, provider-defined input-kind identifiers, optional metadata, and bounded accepted JSON-like values through the core-owned value model. Frozen protocol v1 has no standalone input-description field; presentation belongs in metadata.
- **Tests:** undeclared/missing/duplicate/oversize validation and no mutation API.
- **Done when:** input values cannot influence graph construction in core.
- **Stop:** generic mutable variable/update operation appears.

### T037 [x] Implement append-only evidence model
- **Depends:** T008, T032–T035
- **Files:** `core/src/model/evidence.rs`, tests.
- **Deliver:** stable run-scoped ID, kind, exact bounded opaque locator, digest, media type, metadata, caller/provider source, and immutable record; no URI/path/CWD interpretation.
- **Tests:** empty/control/NUL/oversize rejection, opaque provider-relative convention preservation, association separation, same-locator distinct revisions, no dereference API.
- **Done when:** correction requires new evidence.
- **Stop:** engine interprets provider-specific artifact/workspace semantics.

### T038 [x] Implement graph state and guidance model
- **Depends:** T032–T036
- **Files:** `core/src/model/graph.rs`, `guidance.rs`, tests.
- **Deliver:** flat states, initial ID, final flag, metadata, static text or explicit no-guidance, and stored live-guidance capability. Frozen protocol v1 has no standalone state title/summary fields; presentation beyond state ID/static guidance belongs in metadata.
- **Tests:** zero/one/multiple finals and initial-final representable.
- **Done when:** hierarchy/parallel/timer semantics are unrepresentable.
- **Stop:** candidate input values are accepted by graph constructor.

### T039 [x] Implement transition and gate declarations
- **Depends:** T033, T038
- **Files:** `core/src/model/transition.rs`, tests.
- **Deliver:** source/event/target and unique required gate IDs supporting cycles/self-loops.
- **Tests:** cycle/self-loop/gate-free/multi-gate construction tests.
- **Done when:** provider cannot attach target-state authority to verdict.
- **Stop:** transition selection delegated to provider.

### T040 [x] Implement semantic graph validation
- **Depends:** T038–T039
- **Files:** `core/src/model/graph_validation.rs`, tests.
- **Deliver:** initial/target checks, uniqueness, one `(state,event)`, final sinks, gate uniqueness, guidance declaration, supported semantics.
- **Tests:** complete valid/invalid matrix from testing doctrine.
- **Done when:** cycles, zero-final, multiple-final, initial-final, and non-final sink are valid.
- **Stop:** reachability/DAG policy rejects allowed graphs.

### T041 [x] Implement canonical semantic projection model
- **Depends:** T014, T036, T038–T040
- **Files:** `core/src/model/graph_projection.rs`, `core/tests/graph_projection_golden.rs`.
- **Deliver:** core semantic view exposing all digest-relevant fields without Serde/canonical byte technology.
- **Tests:** reorder-equivalent graphs compare semantically; meaningful field changes differ.
- **Done when:** integration can encode canonical bytes without raw provider JSON.
- **Stop:** hash/canonical serializer leaks into core.

### T042 [x] Implement provider identity and observation model
- **Depends:** T032–T034
- **Files:** `core/src/model/provider.rs`, tests.
- **Deliver:** immutable registration identity, mutable handle/config facts, locator/digest/version observations, and invocation phase facts.
- **Tests:** digest/path/version cannot compare as workflow identity.
- **Done when:** unavailable digest is representable without pretending equality.
- **Stop:** creation locator pins active run.

### T043 [x] Implement gate result model
- **Depends:** T037, T039, T042
- **Files:** `core/src/model/gate.rs`, tests.
- **Deliver:** exact complete verdict set with pass/fail diagnostics/evidence, explicit incompatibility, or evaluation error.
- **Tests:** incompatible/error variants cannot carry evidence; duplicate/missing/extra verdict construction rejected.
- **Done when:** exactly three semantic result variants exist.
- **Stop:** provider result can choose target state.

### T044 [x] Implement compatibility and live-guidance model
- **Depends:** T037–T043
- **Files:** `core/src/model/compatibility.rs`, `live_guidance.rs`, tests.
- **Deliver:** capability-scoped non-latching findings and `LiveGuidanceResult` variants for bounded advisory guidance, explicit stored-guidance incompatibility, or evaluation error, all without evidence/state authority.
- **Tests:** mixed support findings, all three guidance results, and no evidence-bearing guidance.
- **Done when:** compatibility state cannot be persisted as latch through model API.
- **Stop:** registration-wide report mutates every run.

### T045 [x] Implement run aggregate and lifecycle
- **Depends:** T033–T044
- **Files:** `core/src/model/run.rs`, `lifecycle.rs`, tests.
- **Deliver:** stable ID, registration ID, graph snapshot/revision, immutable inputs, current state, active/final/terminated lifecycle, workflow version, optional non-unique label.
- **Tests:** initial-final, active-only label, terminal no-reopen, terminal annotation allowance.
- **Done when:** current state is direct authority and no replay constructor exists.
- **Stop:** run identity depends on workspace/provider path/label.

### T046 [x] Implement immutable journal facts
- **Depends:** T011, T034–T045
- **Files:** `core/src/model/journal.rs`, `attempt.rs`, tests.
- **Deliver:** bounded aggregate entry kinds and required observed facts, bounded association/provider/verdict/diagnostic components, state/version before/after, outcome, correction link; encoded maximum 2.5 MiB.
- **Tests:** contract examples and maximum-component arithmetic construct below aggregate bound; one-byte-over rejects; edit/delete/fold APIs absent.
- **Done when:** journal explains but cannot claim replay.
- **Stop:** current run state is derived from entries.

### T047 [x] Implement public outcome and reason taxonomy
- **Depends:** T006, T032–T046
- **Files:** `core/src/model/outcome.rs`, `reason.rs`, `diagnostic.rs`, tests.
- **Deliver:** implement D004/D006 frozen completed/rejected/error and reason-code catalog, run snapshot, state-changed flag, requestable events, and evidence-recorded status; no new code may originate here.
- **Tests:** every frozen reason maps exactly once; rejected creation maps to no-run/no-journal; self-loop completes unchanged.
- **Done when:** no fourth top-level class exists.
- **Stop:** persistence failure reports evidence/state committed without proof.

### T048 [x] Implement gate-free deterministic transition resolver
- **Depends:** T039–T047
- **Files:** `core/src/model/decision.rs`, tests.
- **Deliver:** active lifecycle/event resolution, unknown/ambiguous rejection, target/final lifecycle, self-loop, requestable-event calculation, and no-provider result.
- **Tests:** table tests for linear/cycle/self-loop/finals/sinks/terminal/unknown.
- **Done when:** identical inputs yield identical decision regardless of actor metadata.
- **Stop:** resolver invokes external capability.

### T049 [x] Implement gated deterministic decision resolver
- **Depends:** T043, T047–T048
- **Files:** `core/src/model/decision.rs`, tests.
- **Deliver:** exact verdict enforcement, fail/incompatibility/evaluation classifications, state preservation, provider evidence handling, and target authority from stored graph only.
- **Tests:** pass/fail/mixed/incompatible/error/malformed-set matrix.
- **Done when:** only complete all-pass result advances.
- **Stop:** provider-reported target or substituted gate accepted.

### T050 [x] Implement requestable-event projection
- **Depends:** T038–T049
- **Files:** `core/src/model/requestable.rs`, tests.
- **Deliver:** current-state event/gate summaries for active run; empty set for terminal; no gate-pass prediction.
- **Tests:** zero-event sink, multi-event, self-loop, terminal cases.
- **Done when:** show/outcomes can share one provider-free projection.
- **Stop:** computation executes provider or compatibility check.

### T051 [x] Add pure core property tests
- **Depends:** T040, T048–T050
- **Files:** `core/tests/model_properties.rs`.
- **Deliver:** no-mock generated graphs/decisions for determinism, rejection preservation, final sinks, and actor neutrality.
- **Tests:** deterministic seed replay command.
- **Done when:** tests supplement rather than claim behavioral authority.
- **Stop:** property model imports outer integrations or replaces E2E commitments.

### T052 [x] Document core model and decision boundaries
- **Depends:** T031–T051
- **Files:** `core/src/lib.rs`, `../../architecture.md`, `../../graph-projection.md`, `../../journal-contract.md`.
- **Deliver:** exact model invariants, deterministic inputs, excluded semantics, and no-replay/no-provider-authority statements.
- **Done when:** core behavior introduced so far is coherent at the T052 publication checkpoint.
- **Stop:** docs claim public operations before exposure tasks.

### V002 [x] Junction validation — core model (T031–T052)
- **Depends:** T052
- **Owner:** orchestrator in V-mode; per junction template above; not a Fable/Sol review round.
- **Files:** none tracked; untracked `target/junction-evidence/V002/**` only.
- **Deliver:** full accumulated inventory plus `cargo doc --workspace --no-deps`, focused core model/property suites, and golden projection tests against the T052 candidate; findings ledger or clean report.
- **Done when:** every command has a recorded outcome and findings are handed to F002 or a clean report is filed; stop for orchestrator.

### F002 [x] Junction fix — batch V002 findings
- **Depends:** V002
- **Owner:** orchestrator in F-mode; one batched pass; per junction template above; not a correctness review round.
- **Files:** files within T031–T052 contracts as valid findings require; changed paths append to the T052 range ledger.
- **Deliver:** one batched repair pass for all valid V002 findings, or explicit no-op completion.
- **Done when:** no-op recorded, or repairs complete and targeted V002 rerun reports clean; the T052 boundary commit and T053 stay blocked until junction closure; stop for orchestrator.

## Phase 3 — Core capabilities and private operations

No capability mocks, fakes, or in-memory port implementations are permitted. Private-operation focused tests cover pure validation/decision functions, command construction, and compile-time capability contracts only. Durable effects and orchestration claims remain explicitly open until production integrations and black-box exposure tasks; Phase 3 checks never count as behavioral acceptance.

### T053 [x] Define time, ID, and digest capabilities
- **Depends:** T031–T052, V002, F002
- **Files:** `core/src/capabilities/{time,id_generator,digest}.rs`.
- **Deliver:** narrow external-effect contracts using core types only.
- **Tests:** compile-only capability contract tests.
- **Done when:** operations need no system clock/UUID/hash dependency.
- **Stop:** generic dependency-injection container proposed.

### T054 [x] Define targeted decision-event capability
- **Depends:** T010, T047–T050
- **Files:** `core/src/capabilities/decision_events.rs`.
- **Deliver:** narrow consequential decision events for transition/gate/lifecycle/compatibility/stale facts, not raw JSON logging.
- **Tests:** architecture compile test; event variants map to trace contract.
- **Done when:** pure helpers require no logging parameter.
- **Stop:** trace context threaded through every model function.

### T055 [x] Define provider catalog capability
- **Depends:** T008, T042, T045, T053
- **Files:** `core/src/capabilities/provider_catalog.rs`.
- **Deliver:** add/list/resolve/update/rename/disable/restore, immutable `ResolvedProviderConfig` carrying config revision, byte/count-paged active-run impact/snapshot queries, active-set digest, and atomic guarded catalog-mutation commands.
- **Criteria:** contract expresses immutable ID, enabled-handle uniqueness, tombstone restore, explicit config, one resolution per provider-dependent operation, and no query-then-mutate TOCTOU API.
- **Done when:** run operations resolve by stored registration ID.
- **Stop:** handle/path/digest becomes identity.

### T056 [x] Define five-role provider invocation capability
- **Depends:** T005, T036–T045, T053
- **Files:** `core/src/capabilities/provider_invoker.rs`.
- **Deliver:** typed five-role calls accepting immutable resolved config plus bounded snapshots/observations and explicit pre-launch trace-budget-unavailable error.
- **Criteria:** invoker exposes no catalog lookup, target-state setter, caller CWD, shell string, retry, environment override, or unbounded payload field; budget error proves provider did not launch.
- **Done when:** all role results preserve three semantic gate variants.
- **Stop:** provider DTO/Serde types cross inward.

### T057 [x] Define provider-free run read capability
- **Depends:** T008, T045–T047
- **Files:** `core/src/capabilities/run_reader.rs`.
- **Deliver:** get/list/show graph/evidence/history and selected-evidence snapshot reads from authoritative state.
- **Criteria:** no provider or journal-replay dependency in signatures.
- **Done when:** active-default and terminal-inclusive filters are representable.
- **Stop:** labels accepted as unique run lookup.

### T058 [x] Define atomic run mutation capability
- **Depends:** T045–T047, T053
- **Files:** `core/src/capabilities/run_writer.rs`, `persistence_commands.rs`.
- **Deliver:** atomic create, evidence, annotation, label, termination, guidance-attempt, and per-run compatibility-attempt commands with journal facts.
- **Criteria:** each command requires complete authority+journal payload and exposes no partial write.
- **Done when:** state mutation cannot bypass matching journal request.
- **Stop:** generic save/update method appears.

### T059 [x] Define atomic event-attempt capability
- **Depends:** T046–T049, T057–T058
- **Files:** `core/src/capabilities/event_attempt_writer.rs`.
- **Deliver:** one command for completed/rejected/error attempts with inline/selected/provider evidence, expected workflow version, state transition, and committed-status result.
- **Criteria:** stale branch can append error attempt without applying transition; persistence failure cannot claim recording.
- **Done when:** provider invocation needs no held write lock.
- **Stop:** label/note/evidence mutation invalidates expected workflow version.

### T060 [x] Define read-only export capability
- **Depends:** T015, T046, T057
- **Files:** `core/src/capabilities/audit_export.rs`.
- **Deliver:** consistent read snapshot contract and explicit output target facts without import/restore API.
- **Criteria:** export operation cannot mutate run/store.
- **Done when:** state/journal/evidence/provider observations are representable.
- **Stop:** JSON DTO annotations enter core.

### T061 [x] Add capability architecture contract tests
- **Depends:** T053–T060
- **Files:** `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/capabilities/*`.
- **Deliver:** compile/dependency checks for core-only types, no integration construction, and operation→capability→model direction.
- **Tests:** architecture canaries for accidental outer leakage.
- **Done when:** accidental outer leakage fails before behavior tests.
- **Stop:** tests depend on source-text logging counts.

### T062 [x] Implement runtime operation catalog and baseline closure tool
- **Depends:** T004, T019, T031, T047
- **Files:** `core/src/operations/catalog.rs`, `xtask/src/operation_coverage.rs`, tests.
- **Deliver:** stable `OperationId`, iteration, uniqueness, facet metadata link, initially empty exposed set, and baseline extensible closure command with core collector plus named collector-registration API and empty-set support. Runtime collectors enumerate only currently registered/exposed IDs; D004's 21-ID table remains target documentation until each exposure and final T167 assertion.
- **Tests:** core-only empty-set pass plus duplicate/missing-core canaries; driver/routes are added by T133, E2E/trace by T145.
- **Done when:** operation can be added only through one reviewed API and planned/unexposed IDs cannot appear in any runtime collector.
- **Stop:** planned-but-unimplemented IDs appear in runtime exposed set.

### T063 [x] Implement private `provider.add` operation
- **Depends:** T047, T053, T055
- **Files:** `core/src/operations/provider_add.rs` (inline tests).
- **Deliver:** validate explicit config/handle, allocate immutable ID, persist through catalog capability.
- **Tests:** pure handle/config validation and `DuplicateHandle` capability-error mapping; durable duplicate/distinct-ID proof remains T146–T147.
- **Done when:** operation remains unexposed until T147.

### T064 [x] Implement private `provider.list` operation
- **Depends:** T008, T047, T055
- **Files:** `core/src/operations/provider_list.rs`, tests.
- **Deliver:** provider-free enabled/tombstoned registration listing plus `--active-runs-for` impact listing under D008 count/byte cursor contract.
- **Tests:** pure filter/cursor validation, page construction, and query/result mapping cover malformed/version/filter/count/byte/no-truncation branches; durable registration/impact paging remains T147/T157/T175.
- **Done when:** operation remains unexposed until T147.

### T065 [x] Implement private `provider.check` operation
- **Depends:** T040–T044, T047, T055–T057
- **Files:** `core/src/operations/provider_check.rs`, tests.
- **Deliver:** explicit protocol/conformance/latest-graph check plus count/byte paging with one `describe` plus at most nine compatibility calls (ten total); resolve config once per page; stable keyset iteration; zero-page completion; pre-launch trace-budget error before first row errors unchanged cursor, after progress returns cursor; other failures error whole page; no truncated finding, journal fan-out, or latch.
- **Tests:** pure zero/mixed/incompatible/result-error aggregation and command construction; production catalog-resolution count and no-journal proof remain T152 provider-ledger E2Es.
- **Done when:** no run is created and operation remains unexposed until atomic T152 closure.

### T066 [x] Implement private `provider.update` operation
- **Depends:** T047, T055
- **Files:** `core/src/operations/provider_update.rs`, tests.
- **Deliver:** atomically replace executable/argv/CWD/timeout under same ID, increment config revision, and return affected count plus `provider.list --active-runs-for` cursor/link without approval flag or unbounded IDs.
- **Tests:** pure command/result mapping preserves ID/config revision; durable linearization remains T107–T119/T178 and current-config use remains T163/T175.
- **Done when:** operation remains unexposed until T155.

### T067 [x] Implement private `provider.rename` operation
- **Depends:** T047, T055
- **Files:** `core/src/operations/provider_rename.rs`, tests.
- **Deliver:** active registration handle rename preserving ID.
- **Tests:** pure handle validation and capability command/error mapping; durable uniqueness and run-binding stability remain T156/T175.
- **Done when:** operation remains unexposed until T156.

### T068 [x] Implement private `provider.disable` operation
- **Depends:** T045, T047, T055
- **Files:** `core/src/operations/provider_disable.rs`, tests.
- **Deliver:** non-mutating warning-page commands where only final page yields opaque full-set/config-bound token, plus atomic `--allow-active-runs <ack-token>` tombstone command preserving ID/revision rules.
- **Tests:** pure first/intermediate cursor cannot authorize; final token maps to guarded command with no query-then-write API; durable proof remains T107–T119/T157/T178.
- **Done when:** operation remains unexposed until T157.

### T069 [x] Implement private `provider.restore` operation
- **Depends:** T068
- **Files:** `core/src/operations/provider_restore.rs`, tests.
- **Deliver:** restore exact tombstoned ID with explicit config and free handle.
- **Tests:** pure restore-command/error mapping preserves ID and classifies occupied handle; durable restore/rejection proof remains T158/T175.
- **Done when:** operation remains unexposed until T158.

### T070 [x] Implement private `run.create` operation
- **Depends:** T040–T047, T053, T055–T059
- **Files:** `core/src/operations/run_create.rs`, tests.
- **Deliver:** resolve once, describe, validate graph, conditionally validate inputs, detect observed digest drift, compute revision, initialize lifecycle, atomically store creation journal.
- **Tests:** pure zero-input skip, input/graph/drift classification, and initial-lifecycle command construction; durable no-run/journal proof remains T149/T152.
- **Done when:** operation remains unexposed until T152.

### T071 [x] Implement private `run.list` operation
- **Depends:** T008, T045, T047, T057
- **Files:** `core/src/operations/run_list.rs`, tests.
- **Deliver:** provider-free active-default and explicit terminal/all keyset-paged listing under D008 cursor contract.
- **Tests:** pure filter/cursor parsing and query-command construction prove no CWD/provider field and no label identity; durable paging remains T150/T152.
- **Done when:** operation remains unexposed until T152.

### T072 [x] Implement private `run.show` operation
- **Depends:** T045, T047, T050, T057
- **Files:** `core/src/operations/run_show.rs`, tests.
- **Deliver:** lifecycle/state/inputs/static guidance/gates/live capability/empty selection/requestable events.
- **Tests:** pure lifecycle projection empties terminal events and command shape has no provider capability; durable read/no-invocation proof remains T153.
- **Done when:** operation remains unexposed until T153.

### T073 [x] Implement private `run.graph` operation
- **Depends:** T041, T045, T047, T057
- **Files:** `core/src/operations/run_graph.rs`, tests.
- **Deliver:** provider-free complete stored projection and graph revision.
- **Tests:** pure projection accepts stored snapshot only; durable drift/missing-provider proof remains T154.
- **Done when:** operation remains unexposed until T154.

### T074 [x] Implement private `run.history` operation
- **Depends:** T008, T046–T047, T057
- **Files:** `core/src/operations/run_history.rs`, tests.
- **Deliver:** ordered provider-free journal query with D008 sequence-keyset pagination/filter contract.
- **Tests:** pure cursor/filter validation and ordered-row projection contain no state reconstruction/provider call; durable history paging remains T152.
- **Done when:** operation remains unexposed until T152.

### T075 [x] Implement private `run.evidence.add` operation
- **Depends:** T037, T046–T047, T053, T058
- **Files:** `core/src/operations/evidence_add.rs`, tests.
- **Deliver:** validate/ID/append evidence and journal atomically on any lifecycle.
- **Tests:** pure locator validation, ID-bearing append-command construction, and lifecycle classification; durable duplicate-locator/terminal atomic append remains T159–T160.
- **Done when:** operation remains unexposed until T160.

### T076 [x] Implement private `run.evidence.list` operation
- **Depends:** T008, T037, T047, T057
- **Files:** `core/src/operations/evidence_list.rs`, tests.
- **Deliver:** provider-free stable keyset-paged inventory with prior event associations under D008 cursor contract.
- **Tests:** pure cursor/filter validation and evidence-row projection are provider-free; durable terminal/missing-provider paging remains T160.
- **Done when:** operation remains unexposed until T160.

### T077 [x] Implement private `run.annotate` operation
- **Depends:** T035, T046–T047, T058
- **Files:** `core/src/operations/run_annotate.rs`, tests.
- **Deliver:** append note/opaque actor/correction journal fact without authority.
- **Tests:** pure note/actor/correction validation and append-command construction preserve state/version fields; durable lifecycle/history proof remains T161.
- **Done when:** operation remains unexposed until T161.

### T078 [x] Implement private `run.label` operation
- **Depends:** T045–T047, T058
- **Files:** `core/src/operations/run_label.rs`, tests.
- **Deliver:** active-only optional non-unique label replacement plus journal.
- **Tests:** pure lifecycle classification and label-command construction preserve run identity/workflow version; durable terminal rejection and rebinding proof remain T162.
- **Done when:** operation remains unexposed until T162.

### T079 [x] Implement private `run.request` orchestration
- **Depends:** T048–T050, T054–T059
- **Files:** `core/src/operations/run_request.rs` (inline tests).
- **Deliver:** run lookup, selection validation, event resolution, gate-free path, batched gated path, decision, CAS attempt commit, accurate evidence status, no retry.
- **Tests:** pure selection/evidence/verdict/stale classification and completed/rejected/error command construction only; durable lookup, CAS, persistence, invocation, and attempt-outcome proof remains T112–T113/T163.
- **Done when:** operation remains unexposed until T163 and no write lock contract spans provider.
- **Stop:** full applicable facets cannot be enumerated in coverage map.

### T080 [x] Implement private `run.guidance` operation
- **Depends:** T044–T047, T054–T059
- **Files:** `core/src/operations/run_guidance.rs`, tests.
- **Deliver:** active/stored-capability checks, explicit provider call, advisory/incompatibility/evaluation result mapping, completed/rejected/error journaling, no evidence/state mutation.
- **Tests:** pure stored-unsupported/incompatibility/terminal/error classification and journal-command construction; no-invocation and post-lookup persistence proof remain T164 production E2Es.
- **Done when:** operation remains unexposed until T164.

### T081 [x] Implement private `run.compatibility` operation
- **Depends:** T044–T047, T054–T058
- **Files:** `core/src/operations/run_compatibility.rs`, tests.
- **Deliver:** active-run explicit current-provider check with mixed non-latching findings and compatibility-attempt journal command carrying actual provider locator/digest/version and observed drift, without state/latch mutation.
- **Tests:** pure finding/lifecycle/drift classification and journal-command construction; durable atomic invocation/history proof remains T114/T165.
- **Done when:** operation remains unexposed until T165.

### T082 [x] Implement private `run.terminate` operation
- **Depends:** T045–T047, T058
- **Files:** `core/src/operations/run_terminate.rs`, tests.
- **Deliver:** active→terminated mutation with optional note and journal, no provider.
- **Tests:** pure lifecycle classification and versioned termination-command construction; durable repeat/final rejection, no-reopen, increment, and journal proof remains T151–T152.
- **Done when:** operation remains unexposed until T152.

### T083 [x] Implement private `run.export` operation
- **Depends:** T015, T047, T060
- **Files:** `core/src/operations/run_export.rs`, tests.
- **Deliver:** consistent read-only audit snapshot request to explicit target.
- **Tests:** pure request/command shape exposes no import/mutation/replay/dereference path; durable export safety remains T166/T181.
- **Done when:** operation remains unexposed until T166.

### V003 [x] Junction validation — private operations (T053–T083)
- **Depends:** T083
- **Owner:** orchestrator in V-mode; per junction template above; not a Fable/Sol review round.
- **Files:** none tracked; untracked `target/junction-evidence/V003/**` only.
- **Deliver:** full accumulated inventory plus `cargo run -p xtask -- operation-coverage --mode baseline` and focused private-operation test modules against the T083 candidate; findings ledger or clean report.
- **Done when:** every command has a recorded outcome and findings are handed to F003 or a clean report is filed; stop for orchestrator.

### F003 [x] Junction fix — batch V003 findings
- **Depends:** V003
- **Owner:** orchestrator in F-mode; one batched pass; per junction template above; not a correctness review round.
- **Files:** files within T053–T083 contracts as valid findings require; changed paths append to the T083 range ledger.
- **Deliver:** one batched repair pass for all valid V003 findings, or explicit no-op completion.
- **Done when:** no-op recorded, or repairs complete and targeted V003 rerun reports clean; the T083 boundary commit and T084 stay blocked until junction closure; stop for orchestrator.

## Phase 4 — Provider, configuration, trace, and system integrations

### T084 [x] Define provider protocol and published schemas
- **Depends:** T005, T008, T014, T017, T031–T050, V003, F003
- **Files:** `integrations/src/provider_protocol/{mod,dto,version}.rs`, `schemas/provider/v1/*.json`.
- **Deliver:** request/result envelopes and five operation schemas with bounds and same-major compatibility.
- **Tests:** schema golden fixtures for every valid result variant and malformed/unsupported cases.
- **Done when:** schema contains no state-setter and input validation cannot emit topology.

### T085 [x] Implement common provider DTO mapping
- **Depends:** T084
- **Files:** `integrations/src/provider_protocol/mapping.rs`, `validation.rs`, tests.
- **Deliver:** external DTO↔core conversion with structural bounds and actionable paths.
- **Tests:** unknown optional fields accepted; unknown required semantics, oversize, wrong tags, and malformed metadata fail.
- **Done when:** Serde/Schemars types remain outside core.

### T086 [x] Implement graph mapping, semantic validation, and canonical bytes
- **Depends:** T014, T040–T041, T084–T085
- **Files:** `integrations/src/provider_protocol/{graph,canonical}.rs`, `integrations/tests/graph_canonical.rs`, `integrations/tests/fixtures/graphs/*.json`.
- **Deliver:** DTO→validated core graph→canonical DTO/bytes→SHA-256 input.
- **Tests:** reordering equivalence and every digest-relevant field-change vector.
- **Done when:** raw provider JSON formatting cannot affect graph revision.

### T087 [x] Implement subprocess spawn and lifecycle
- **Depends:** T002, T005, T007–T008, T056, T084
- **Files:** `integrations/src/provider_process/{mod,spawn,error}.rs`, tests.
- **Deliver:** literal executable/argv, explicit CWD, inherited caller environment with no registration overrides, stdin request, closed EOF, status/signal classification, no shell.
- **Tests:** caller-CWD independence, provider sees inherited test sentinel, missing executable, nonzero, signal, and no environment field/value appears in trace payload.
- **Done when:** arbitrary provider output never reaches CLI streams directly.

### T088 [x] Implement deadlock-safe bounded stream capture
- **Depends:** T087
- **Files:** `integrations/src/provider_process/streams.rs`, tests.
- **Deliver:** concurrent stdout/stderr drain, byte bounds, UTF-8/error policy, exact one-result extraction.
- **Tests:** simultaneous full streams, oversize stdout/stderr, missing result, extra stdout, invalid UTF-8.
- **Done when:** provider cannot deadlock on full pipe.

### T089 [x] Implement provider timeout and process-group termination
- **Depends:** T002, T087–T088
- **Files:** `integrations/src/provider_process/{timeout,process_group}.rs`, `integrations/tests/provider_timeout.rs`.
- **Deliver:** configurable deadline, process-group kill, child cleanup, timeout observation, no retry.
- **Tests:** real blocking provider and descendant process terminate within test budget.
- **Done when:** no orphan survives supported-platform timeout.
- **Stop:** platform cannot support selected semantics; reopen D002/D005.

### T090 [x] Implement executable digest observation
- **Depends:** T053, T087
- **Files:** `integrations/src/provider_process/digest.rs`, tests.
- **Deliver:** best-effort pre-invocation SHA-256 and normalized actual locator; unavailable digest is explicit.
- **Tests:** readable, unreadable, replaced, and interpreted-script cases.
- **Done when:** digest is audit fact, never identity or environment proof.

### T091 [x] Implement provider `describe` adapter
- **Depends:** T085–T090
- **Files:** `integrations/src/provider_protocol/describe.rs`, tests.
- **Deliver:** input-free request, graph/declarations/guidance mapping, observation capture.
- **Tests:** candidate inputs absent; malformed/invalid result classified.
- **Done when:** complete graph can be validated and canonicalized.

### T092 [x] Implement provider input-validation adapter
- **Depends:** T036, T085–T090
- **Files:** `integrations/src/provider_protocol/validate_inputs.rs`, tests.
- **Deliver:** value-only request and accepted/rejected/error mapping with topology-output rejection.
- **Tests:** zero-input caller path can skip adapter; invalid values remain domain rejection.
- **Done when:** description/validation digest observations can be compared.

### T093 [x] Implement provider gate-evaluation adapter
- **Depends:** T037, T043, T085–T090
- **Files:** `integrations/src/provider_protocol/evaluate_gates.rs`, tests.
- **Deliver:** bounded snapshot, complete requested gate set, three result variants, provider evidence validation.
- **Tests:** missing/extra/duplicate/substituted verdict, invalid evidence, incompatibility-with-evidence, evaluation-error-with-evidence.
- **Done when:** only complete verdict result can carry valid evidence.

### T094 [x] Implement provider live-guidance adapter
- **Depends:** T044, T085–T090
- **Files:** `integrations/src/provider_protocol/live_guidance.rs`, tests.
- **Deliver:** bounded active-run context and exactly one result: advisory guidance, explicit stored-guidance incompatibility, or evaluation error.
- **Tests:** incompatibility maps distinctly; evidence/state fields rejected; guidance bound enforced.
- **Done when:** adapter cannot authorize transition.

### T095 [x] Implement provider compatibility adapter
- **Depends:** T044, T085–T090
- **Files:** `integrations/src/provider_protocol/compatibility.rs`, tests.
- **Deliver:** stored graph/declaration capability request and mixed finding mapping.
- **Tests:** non-latching support/incompatibility/error cases.
- **Done when:** latest graph is not substituted for stored run graph.

### T096 [x] Implement machine-local path resolver
- **Depends:** T002, T007, T030
- **Files:** `integrations/src/configuration/paths.rs`, tests.
- **Deliver:** OS roots, `LOOP_ENGINE_HOME`, DB/trace/global/project locations, ancestor search.
- **Tests:** isolated home, root boundary, symlink policy, another CWD, no project file.
- **Done when:** paths are deterministic and provider discovery never occurs.

### T097 [x] Implement typed TOML configuration DTOs
- **Depends:** T007–T008, T096
- **Files:** `integrations/src/configuration/{dto,load,error}.rs`, tests.
- **Deliver:** global/project defaults parsing, unknown-key policy, malformed diagnostics, no registration definitions in project file.
- **Tests:** valid/malformed/unsupported/forbidden-key fixtures.
- **Done when:** external config DTOs do not leak inward.

### T098 [x] Implement configuration precedence and registration defaults
- **Depends:** T097
- **Files:** `integrations/src/configuration/resolved.rs`, tests.
- **Deliver:** CLI > project > global > built-in merge for allowed defaults/references.
- **Tests:** full precedence matrix and existing-run stored-ID bypass of defaults.
- **Done when:** caller CWD cannot alter active-run provider config.

### T099 [x] Implement trace event DTO schema
- **Depends:** T010, T023, T047, T054
- **Files:** `integrations/src/trace/{event,schema}.rs`, `schemas/trace/v1/*.json`, `schemas/trace/v1/fixtures/*.json`.
- **Deliver:** versioned dispatcher/provider/persistence/decision event DTOs and payload bound markers.
- **Tests:** schema fixtures for start/result/error/rollback/stale/crash plus exact on-disk maximum encoding for control-heavy JSON and binary/base64 streams; raw/parsed duplication canary fails.
- **Done when:** events remain diagnostics, not replay/journal records.

### T100 [x] Implement secure per-invocation trace writer
- **Depends:** T002, T007–T008, T099
- **Files:** `integrations/src/trace/{mod,writer,error}.rs`, tests.
- **Deliver:** unique file, current-user-only creation, request ID, buffered/flush policy, init failure.
- **Tests:** Unix mode tests, collisions, concurrent files, write/flush failures.
- **Done when:** init failure is distinguishable before dispatch.

### T101 [x] Implement cross-process trace rotation
- **Depends:** T100
- **Files:** `integrations/src/trace/rotation.rs`, tests.
- **Deliver:** 128 MiB cap over encoded actual plus unused reservation remainder, atomic write conversion from reserved→actual, 16 MiB base/10 MiB call additions, cross-process coordination, deterministic closed victim, 120 MiB active cap.
- **Tests:** maximum ten-call arithmetic never double-counts; reservation consume/exhaust/release and concurrent writers never exceed 128 MiB; active preservation/stale-lock recovery.
- **Done when:** hard directory bound and complete launched-call trace coexist under concurrency.

### T102 [x] Implement traced provider boundary wrapper
- **Depends:** T087–T101
- **Files:** `integrations/src/provider_process/traced.rs`, tests.
- **Deliver:** acquire 10 MiB encoded-call reservation before launch; embed request JSON once, base64 each raw stream once, avoid parsed-result duplication, record facts/timing/correlation, consume reservation as writes land, release only unused remainder at trace close.
- **Tests:** maximum encoded JSON/control/binary fixtures fit reservation; one-byte-over cannot launch; all failures emit last observable event; inherited environment absent.
- **Done when:** no alternate production provider path bypasses wrapper.

### T103 [x] Implement system clock, ID, and SHA-256 adapters
- **Depends:** T053, T017
- **Files:** `integrations/src/{system_clock,uuid_ids,sha256_digest}.rs`, tests.
- **Deliver:** UTC timestamps, selected IDs, graph/executable digest implementation.
- **Tests:** format, concurrency uniqueness, known SHA vectors.
- **Done when:** graph and executable digest types remain distinct.

### V004 [x] Junction validation — provider/config/trace integrations (T084–T103)
- **Depends:** T103
- **Owner:** orchestrator in V-mode; per junction template above; not a Fable/Sol review round.
- **Files:** none tracked; untracked `target/junction-evidence/V004/**` only.
- **Deliver:** full accumulated inventory plus focused integrations suites (protocol schema fixtures, subprocess/timeout/digest, configuration, trace writer/rotation/traced-boundary) against the T103 candidate; findings ledger or clean report.
- **Done when:** every command has a recorded outcome and findings are handed to F004 or a clean report is filed; stop for orchestrator.

### F004 [x] Junction fix — batch V004 findings
- **Depends:** V004
- **Owner:** orchestrator in F-mode; one batched pass; per junction template above; not a correctness review round.
- **Files:** files within T084–T103 contracts as valid findings require; changed paths append to the T103 range ledger.
- **Deliver:** one batched repair pass for all valid V004 findings, or explicit no-op completion.
- **Done when:** no-op recorded, or repairs complete and targeted V004 rerun reports clean; the T103 boundary commit and T104 stay blocked until junction closure; stop for orchestrator.

## Phase 5 — SQLite authority, journal, and export

### T104 [x] Implement SQLite open and migration runner
- **Depends:** T001, T007, T009, T030, T096, V004, F004
- **Files:** `integrations/src/persistence/{mod,sqlite,migrations,error}.rs`.
- **Deliver:** bundled open, pragmas, busy policy, transactional ordered migrations, future-schema refusal.
- **Tests:** empty/latest/concurrent open, failed migration rollback, unsupported future version.
- **Done when:** migration execution is traced-capable but independent of CLI.

### T105 [x] Design and freeze migration `0001`
- **Depends:** T009, T011, T014, T036–T046, T055–T060, T104
- **Files:** `integrations/migrations/0001_initial.sql`, `../../persistence.md`, `../../persistence-schema.md`.
- **Deliver:** registrations with config revision, runs with resolved registration revision, snapshots/inputs, evidence, associations, journal, sequences, workflow versions, constraints, indexes.
- **Tests:** schema review against every atomic/stale/terminal/tombstone invariant and SQL integrity tests.
- **Done when:** query authority is direct and no run-delete path exists.
- **Stop:** stale attempt/evidence association cannot be represented atomically.

### T106 [x] Implement persistence record DTO mappings
- **Depends:** T041–T047, T105
- **Files:** `integrations/src/persistence/{records,mapping}.rs`, tests.
- **Deliver:** integration-owned versioned JSON/row DTOs and corruption-validating core mappings.
- **Tests:** round trips plus unknown version/enum, malformed JSON, graph digest mismatch, invalid state/lifecycle.
- **Done when:** core has no Rusqlite/Serde annotations.

### T107 [x] Implement provider catalog persistence
- **Depends:** T055, T104–T106
- **Files:** `integrations/src/persistence/provider_catalog.rs`, tests.
- **Deliver:** add/list/resolve plus config revision, count/byte-paged impact, final-page acknowledgement token binding, atomic guarded update/disable/restore, and handle uniqueness.
- **Tests:** concurrent handle claims; both run-create/catalog-mutation writer orders; tombstone release; exact-ID restore; handle reuse without rebinding.
- **Done when:** registration ID remains immutable and referenced tombstone retained.

### T108 [x] Implement atomic run-creation transaction
- **Depends:** T058, T103–T107
- **Files:** `integrations/src/persistence/run_create.rs`, tests.
- **Deliver:** atomically recheck enabled registration/config revision then commit run/snapshot/inputs/state/version/registration/creation journal.
- **Tests:** initial-final; update/disable/restore between resolve and commit yields stale-provider-config error/no run; injected failure at each write boundary leaves all-or-nothing state.
- **Done when:** rejected/error/stale-config creation writes no run journal.

### T109 [x] Implement provider-free run read queries
- **Depends:** T057, T105–T106
- **Files:** `integrations/src/persistence/run_reads.rs`, tests.
- **Deliver:** get/list/show/graph with active/terminal filters and authoritative columns.
- **Tests:** fresh connection restart, caller CWD irrelevance, no journal replay.
- **Done when:** missing provider has no effect on reads.

### T110 [x] Implement evidence inventory and selected-context reads
- **Depends:** T037, T057, T105–T106
- **Files:** `integrations/src/persistence/evidence_reads.rs`, tests.
- **Deliver:** ordered inventory, associations, exact ID selection, complete context bound check.
- **Tests:** missing ID, unrelated history exclusion, empty default, no truncation.
- **Done when:** provider receives only selected existing plus inline evidence.

### T111 [x] Implement evidence, annotation, label, and termination transactions
- **Depends:** T058, T105–T106
- **Files:** `integrations/src/persistence/run_mutations.rs`, tests.
- **Deliver:** operation-specific atomic writes with journal, lifecycle rules, stable IDs, version behavior.
- **Tests:** fault at each boundary; terminal evidence/annotation; terminal label/termination rejection.
- **Done when:** label/note/evidence do not change workflow CAS version.

### T112 [x] Implement atomic event-attempt transaction
- **Depends:** T059, T105–T106, T110
- **Files:** `integrations/src/persistence/event_attempt.rs`, tests.
- **Deliver:** completed/rejected/error entry, evidence records/associations, provider/gate facts, optional mutation in one transaction; validate component and 2.5 MiB aggregate journal bounds before write.
- **Tests:** all failure-injection points, aggregate-overflow rejection with no partial write, and accurate evidence-recorded result.
- **Done when:** partial attempt/state/evidence is impossible.

### T113 [x] Implement stale-evaluation CAS branch
- **Depends:** T112
- **Files:** `integrations/src/persistence/event_attempt.rs`, `integrations/tests/event_attempt_concurrency.rs`.
- **Deliver:** recheck workflow-state/lifecycle version after provider; apply transition only on match; append stale error attempt without transition when possible.
- **Tests:** two explicit-barrier requests; transition/termination invalidates; label/note/evidence does not.
- **Done when:** no write lock is held across provider invocation.

### T114 [x] Implement guidance and per-run compatibility attempt persistence
- **Depends:** T058–T059, T081, T105–T106
- **Files:** `integrations/src/persistence/{guidance_attempt,compatibility_attempt}.rs` (inline tests).
- **Deliver:** completed/unsupported/lifecycle/error guidance facts plus completed/error compatibility findings, actual provider observations, and drift facts; no evidence/state/latch mutation.
- **Tests:** atomic append, persistence failure, missing provider, unchanged state/version, and drift observation; registration-wide check never uses this writer.
- **Done when:** every post-lookup guidance/compatibility request is explained when persistence remains available.

### T115 [x] Implement ordered history query
- **Depends:** T046, T057, T105–T106
- **Files:** `integrations/src/persistence/history.rs`, tests.
- **Deliver:** immutable per-run sequence query, count/3 MiB byte pagination, correction links, and one-record progress guarantee.
- **Tests:** ordering across processes, no gaps after rollback, corruption detection, and maximum 2.5 MiB golden entry fits one history page/4 MiB CLI envelope without truncation.
- **Done when:** history never derives current state.

### T116 [x] Implement consistent read-only audit export
- **Depends:** T015, T060, T105–T106, T109–T115
- **Files:** `integrations/src/export/{mod,state_json,journal_jsonl}.rs`, `schemas/export/v1/*.json`, `integrations/tests/export.rs`.
- **Deliver:** new/empty directory write from one consistent read snapshot, versioned state/journal files, no import.
- **Tests:** ordering, overwrite rejection, rollback/cleanup on partial filesystem failure, external locators not dereferenced.
- **Done when:** export is not competing authority.

### T117 [x] Implement persistence corruption diagnostics
- **Depends:** T106–T116
- **Files:** `integrations/src/persistence/corruption.rs`, `integrations/tests/corruption.rs`, `integrations/tests/fixtures/corruption/*`.
- **Deliver:** rich errors for malformed DB/header/rows/snapshots/associations/sequences/schema versions.
- **Tests:** no silent repair/default/delete; fresh CLI/export logical authority, schema version, integrity, and row inventory unchanged after failed reads/writes; physical byte hash used only for immutable copied fixtures, never live WAL database.
- **Done when:** failure phase and trace correlation can be rendered.

### T118 [x] Implement traced persistence boundary wrapper
- **Depends:** T099–T101, T104–T117
- **Files:** `integrations/src/persistence/traced.rs`, tests.
- **Deliver:** transaction intent, CAS/version check, commit/rollback/error events and bounded payloads.
- **Tests:** every production write path passes wrapper; read/migration categories follow trace contract.
- **Done when:** no direct production transaction bypass exists.

### T119 [x] Add SQLite overlap and locking integration tests
- **Depends:** T107–T118
- **Files:** `integrations/tests/sqlite_overlap.rs`.
- **Deliver:** independent-run writes, handle races, run-create versus update/disable/restore in both writer orders, busy behavior, migration races, stale CAS, and post-kill reopen tests with explicit barriers.
- **Tests:** no query-then-mutate TOCTOU survives; repeated-run execution under selected CI platforms belongs to junction validation.
- **Done when:** tests use no timing-only sleep ordering.

### V005 [x] Junction validation — persistence authority (T104–T119)
- **Depends:** T119
- **Owner:** orchestrator in V-mode; per junction template above; not a Fable/Sol review round.
- **Files:** none tracked; untracked `target/junction-evidence/V005/**` only.
- **Deliver:** full accumulated inventory plus focused persistence suites (migration, catalog, run-create, mutations, event-attempt/CAS concurrency, history, export, corruption, traced boundary, SQLite overlap) against the T119 candidate; findings ledger or clean report.
- **Done when:** every command has a recorded outcome and findings are handed to F005 or a clean report is filed; stop for orchestrator.

### F005 [x] Junction fix — batch V005 findings
- **Depends:** V005
- **Owner:** orchestrator in F-mode; one batched pass; per junction template above; not a correctness review round.
- **Files:** files within T104–T119 contracts as valid findings require; changed paths append to the T119 range ledger.
- **Deliver:** one batched repair pass for all valid V005 findings, or explicit no-op completion.
- **Done when:** no-op recorded, or repairs complete and targeted V005 rerun reports clean; the T119 boundary commit and T120 stay blocked until junction closure; stop for orchestrator.

## Amended execution plan (2026-07-22)

Everything below supersedes the original Phase 6–11 / T120–T200 microtask structure. Two independent blind assessments ([overengineering-assessment.md](overengineering-assessment.md), [overengineering-assessment-sol.md](overengineering-assessment-sol.md)) converged: the substrate through Phase 5 is sound, but the remaining plan spent a large share of effort on per-task governance instead of shipping runnable behavior. Owner accepted the amendment on 2026-07-22. Planning lessons are distilled in [development-policy.md § Planning and execution lessons](../../development-policy.md#planning-and-execution-lessons-2026-07-22).

Original per-task contracts (T120–T200, V006–V013, F006–F013) remain readable at commit `d397d89` and are referenced by ID below. They stay the contracts of record for deliverable detail wherever a checklist line does not amend them.

Amendment summary:

- remaining work executes as six work packages WP1–WP6, not per-task execution units;
- junctions V006–V013/F006–F013 are retired; each work package ends with one validation ritual and one or few authorized boundary commits;
- operation exposure is staged: nine alpha operations publish first; twelve operations defer to WP6;
- provider process-failure facets are proven exhaustively once through one representative operation (shared family), per amended [testing.md § Facet matrix](../../testing.md#facet-matrix) and [I30](../../invariants.md#i30-end-to-end-depth-follows-operation-facets);
- model-based black-box testing (was T183) defers to the WP6 backlog as an owner-optional item;
- publication ritual, semantic-judge gate, deterministic quality inventory, and all runtime invariants are unchanged.

### Work-package governance

- The orchestrator implements each work package as one continuous assignment; internal checklist order is advisory, stated dependencies are real.
- At each work-package boundary: run `git diff --check` plus `cargo run -p xtask -- quality --root .`; repair failures; commit through the README boundary ritual. Publication uses the unchanged judge-gated pre-push gate.
- The orchestrator stops for explicit owner review at every work-package boundary; unattended multi-package runs are not permitted.
- Blind Fable/Sol correctness review is owner-invoked at boundaries, no longer scheduled per range.
- Uncommitted work must not span more than one work package.
- Closure stages: WP3 checkpoints use `candidate`/`exposed` as marked; `final` (D004 21-ID equality) applies only at WP6 change close.

### Alpha operation scope

Alpha catalog, exposed by WP3 (9 operations):

`provider.add`, `provider.list`, `provider.check`, `run.create`, `run.list`, `run.show`, `run.terminate`, `run.request`, `run.history`.

Deferred to WP6 (12 operations; their private core implementations from Phase 3 remain complete and tested):

`provider.update`, `provider.rename`, `provider.disable`, `provider.restore`, `run.graph`, `run.evidence.add`, `run.evidence.list`, `run.annotate`, `run.label`, `run.guidance`, `run.compatibility`, `run.export`.

`provider.list` and `run.terminate` join the seven-operation assessment slice because list is the fresh-process verification read for every catalog facet and terminate completes the normative lifecycle-family owner set (`run.list`/`run.show`/`run.terminate`).

Alpha facet manifests enumerate alpha-applicable rows only. Facet rows that depend on a deferred operation (selected/provider evidence for `run.request`, updated/restored-config invocation, disable-tombstone interactions) enter the manifest in WP6 together with their operations.

### WP1 — CLI delivery substrate (was Phase 6, T120–T134) — complete

Boundary commit `feat(cli): add private driver and contracts`. The working tree already contains substantial progress on startup, args, diagnostics, and JSON rendering; finish in place, do not restart.

- [x] Trace-first CLI startup: request ID and secure trace before help/version/parse/dispatch per D010; rich init failure stderr (was T120) — `cli/src/{main,startup}.rs`, `cli/tests/startup.rs`
- [x] Shared argument primitives: global flags, private operation parser modules; production root registers no application subcommand (was T121) — `cli/src/args.rs`, `cli/tests/args.rs`
- [x] CLI-to-core request DTO mappings; invalid-syntax versus domain-rejection boundary (was T122) — `cli/src/commands/{mod,provider,run,evidence,export}.rs`
- [x] Sole composition root with architecture bypass canaries (was T123) — `cli/src/composition.rs`
- [x] Traced operation dispatcher: one operation per intent, request/outcome envelopes, correlation (was T124) — `cli/src/dispatch.rs`
- [x] Structured outcome schema and renderer; versioned one-object envelope (was T125) — `cli/src/render/`, `schemas/cli/v1/outcome.schema.json`
- [x] Human renderer with semantic parity against structured renderer (was T126) — `cli/src/render/human.rs`
- [x] Rich diagnostics and source-chain rendering (was T127) — `cli/src/diagnostics.rs`
- [x] Stable exit codes and stdout/stderr behavior, byte-level tests (was T128) — `cli/src/exit.rs`, `cli/tests/process_contract.rs`
- [x] Private command adapters for all 21 operations; none registered in production root (was T129–T132) — `cli/src/commands/*.rs`
- [x] Production driver operation catalog, initially empty, updated only by exposures; closure tool gains driver/route sets (was T133) — `cli/src/driver_catalog.rs`
- [x] Generated/validated provider/trace/export schemas plus planned CLI schema and contract docs (was T134) — `schemas/index.json`, `../../cli-contract.md`, `../../operational-trace.md`, `../../export-contract.md`

Validation (was V006/F006): boundary ritual plus focused startup/args/dispatch/render/exit suites and `baseline` closure with driver/route sets. Final WP1 inventory and post-marker boundary checks passed 19/19 commands and focused CLI suites passed 185/185 tests; evidence is retained under `target/junction-evidence/WP1/`. [`c6-range-ledger.txt`](c6-range-ledger.txt) records the exact boundary union. Owner approved the WP1 boundary on 2026-07-22.

### WP2 — Provider fixtures and E2E harness (was Phase 7, T135–T145)

Boundary commit `test(e2e): establish black-box harness`.

- [ ] Generic scenario-provider executable, root-excluded, no product dependency (was T135) — `test-support/providers/scenario-provider/`
- [ ] Graph/input scenario modes: linear/cycle/self-loop/zero-final/multi-final/initial-final/sink/ambiguous/invalid/guidance/input variants (was T136)
- [ ] Gate/evidence/guidance/compatibility scenario modes (was T137)
- [ ] Provider process-failure modes: malformed JSON, extra/missing output, wrong major, nonzero, signal, timeout, oversized streams, invalid UTF-8 (was T138)
- [ ] Explicit provider barrier and invocation ledger; no timing-only sleeps (was T139)
- [ ] Reference software-change provider: graph, gate/evidence policy, guidance/drift/compatibility modes (was T140–T142) — `test-support/providers/reference-provider/`
- [ ] Isolated E2E sandbox: private home/config/DB/trace/provider CWD, preserved failure artifacts (was T143) — `cli/tests/e2e.rs`, `cli/tests/support/`
- [ ] CLI process runner and structured parser; no in-process handler calls (was T144)
- [ ] Trace parser, runtime coverage recorder, fixture helpers; close C2 (was T145)

Validation (was V007/F007): boundary ritual plus standalone fixture-crate tests and harness self-tests.

### WP3 — Alpha operation exposure (was Phase 8, trimmed to alpha catalog)

Exposure rules kept from the original phase: each checkpoint registers routes and catalog IDs in a deliberately uncommitted candidate tree, closes its alpha facet manifest (`quality/facets/v1/<operation-id>.json`) with exact E2E/trace evidence, updates docs/coverage, then stops for the authorized commit. Checkpoint groups are indivisible same-owner assignments. `Trace persistence boundary` closes from each exposure's production CLI trace; provider users also close `Trace provider boundary`.

- [ ] Checkpoint A — `provider.add` + `provider.list` (was T146–T147): registration, stable ID, duplicate-handle rejection, enabled/tombstoned listing, D008 pagination rows, fresh-list verification. Commit `feat(provider): expose provider catalog foundation`. Closure: `candidate` then `exposed`.
- [ ] Checkpoint B — `provider.check`, `run.create`, `run.list`, `run.terminate`, `run.history` (was T148–T152): provider.check closes the complete shared provider process-failure family as the representative operation (missing executable, timeout, crash/nonzero/signal, malformed/wrong-major/invalid-UTF-8 protocol, oversized output/streams); run.create covers zero/valid/invalid inputs, description/validation drift, rejected/error creation writing no run/journal, plus two representative provider-failure rows (timeout, malformed) referencing the shared family for the rest; run.list/run.terminate/run.history cover lifecycle visibility, termination note/repeat denial, ordered history with D008 cursor rows, fresh-process state/journal proofs. Commit `feat(run): expose provider check and run foundation`. Closure: staged `candidate` per operation, `exposed` at commit.
- [ ] Checkpoint C — `run.show` (was T153): active, neutral final, initial-final, zero-final ongoing, non-final sink, terminated, gates, empty events, missing provider/not-found; provider ledger stays empty. Commit `feat(run): expose run.show`.
- [ ] Checkpoint D — `run.request` (was T163, alpha facets): gate-free/gated flows, all verdict variants, inline evidence, unknown/final/terminated rejection, self-loop/cycle/final semantics, missing/tombstoned provider policy, stale CAS, exact journal/trace, two representative provider-failure rows referencing the shared family. Selected/provider-evidence and updated/restored-config rows defer to WP6. Commit `feat(run): expose run.request`.

Validation (was V008–V010/F008–F010): boundary ritual after each checkpoint commit; full CLI E2E suite plus `exposed` closure.

### WP4 — Cross-operation acceptance (was Phase 9, 18 tasks → 5)

Boundary commit `test(e2e): close alpha acceptance`.

- [ ] A1 — Universal operation/driver/E2E/trace closure over the alpha catalog with missing driver/E2E/trace/open-facet canaries; excluded-API and architecture audit criteria folded in (was T167 + T184) — `cli/tests/e2e/coverage_closure.rs`, `xtask/src/operation_coverage.rs`
- [ ] A2 — Shared provider execution failure family: full cross-product through the representative operation plus one representative row per other provider-invoking alpha operation; creation error proves no run/journal; gate-free operations stay usable (was T170, amended) — `cli/tests/e2e/provider_failures.rs`
- [ ] A3 — Atomicity, migration, corruption: fault-injection rollback at every alpha durable-mutation boundary; frozen migration v1 fixture discipline; corruption cases observed through the public CLI; creation-input and canonical-graph-drift rows folded in (was T173, T179, T180, T171) — `cli/tests/e2e/{atomicity,migrations,corruption}.rs`, `test-support/sqlite/`
- [ ] A4 — Lifecycle, graph semantics, outcome taxonomy: complete lifecycle family across fresh processes; linear/cycles/self-loops/finals/sink/ambiguity graph family; human/structured parity and exit taxonomy (was T174, T168, T169) — `cli/tests/e2e/{lifecycle,graph_semantics,outcomes}.rs`
- [ ] A5 — Trace contract, configuration, concurrency: trace permissions/rotation/exhaustion/late-sink cases for alpha operations; config precedence and caller-CWD independence; overlap/stale-CAS scenarios with explicit barriers (was T182, T177, T178, trimmed to alpha ops) — `cli/tests/e2e/{trace_contract,trace_resilience,configuration,concurrency}.rs`

Deferred with their operations to WP6: evidence flow (was T172), provider drift/identity (was T175), compatibility continuity (was T176), audit export (was T181). Deferred as owner-optional: model-based testing (was T183).

### WP5 — Alpha publication and dogfood

- [ ] User/operator documentation for the exposed alpha surface: root README, `../../cli-contract.md`, `../../configuration.md`, `../../operational-trace.md`, `../../persistence.md` (alpha subset of T193); docs claim no unexposed command
- [ ] Full quality gate green; judge-gated publication of the alpha range through the canonical pre-push ritual
- [ ] Owner dogfoods the reference software-change workflow manually through the alpha CLI; findings and pain points recorded here and used to order WP6

### WP6 — Post-alpha completion backlog

The change stays open through WP6; ordering is an owner decision after dogfood. Contents, reusing original contracts by ID:

- [ ] Deferred exposures with facet manifests: `run.graph` (T154), `provider.update` (T155), `provider.rename` (T156), `provider.disable` (T157), `provider.restore` (T158), `run.evidence.add`/`run.evidence.list` (T159–T160), `run.annotate` (T161), `run.label` (T162), `run.guidance` (T164), `run.compatibility` (T165), `run.export` (T166); deferred `run.request` facet rows close here
- [ ] Deferred acceptance families: evidence flow (T172), provider drift and stable registration identity (T175), compatibility continuity (T176), audit export (T181)
- [ ] Reference acceptance behaviors 1–21 and acceptance report (T185–T190); requires deferred operations
- [ ] Model-based black-box testing (T183) — owner-optional
- [ ] Invariant/facet evidence report (T191); provider-author, operator, and migration/recovery documentation finalization (T192–T194)
- [ ] Hardening at owner-chosen depth: runtime budget/sharding (T196), flake/leak audit (T197), repository protection (T198), clean-room acceptance audit (T199)
- [ ] Change close (was T200): all work packages complete, `final` closure equals D004's 21 IDs, generated evidence linked, owner approves, protected remote accepts publication
