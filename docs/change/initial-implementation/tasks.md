# Initial Implementation Task List

**Status:** In progress — Phase 0

Follow [execution protocol](README.md#execution-protocol), resolve [decision gates](decisions.md), and maintain [coverage map](coverage.md). Every task is sized for one fresh subagent unless its stop condition requires owner escalation.

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

Verification command conventions:

- focused core test: `cargo test -p loop-engine-core <module-filter>`;
- focused integration test: `cargo test -p loop-engine-integrations <module-filter>`;
- targeted CLI E2E: `cargo test -p loop-engine-cli --test e2e <task-listed-filter>`;
- targeted reference E2E: `cargo test -p loop-engine-cli --test reference <task-listed-filter>`;
- operation closure: `cargo run -p xtask -- operation-coverage`;
- architecture: `cargo run -p xtask -- architecture`;
- documentation: `cargo run -p xtask -- docs-check`;
- full gate: `cargo run -p xtask -- quality --revision HEAD`.

A task saying “targeted E2E” means exact CLI E2E command above with operation/scenario name as filter. A task saying “targeted reference E2E” means exact reference command above with behavior-group module as filter. `cli/tests/e2e/*.rs` are modules of `cli/tests/e2e.rs`; `cli/tests/reference/*.rs` are modules of `cli/tests/reference.rs`. For focused core/integration tests, module filter is exact basename of task's primary Rust module (for example T043 `gate`, T105 `migrate`). Closure has four explicit stages: `baseline` requires all runtime sets empty; `candidate --allow-open <comma-separated-stable-ids>` requires core/driver/route equality for named uncommitted atomic-group IDs while permitting their declared open facets; default `exposed` requires exact core=driver=route=E2E=trace equality and closed manifest for every currently exposed ID; `final` adds exact D004 21-ID equality. Bare “closure” means `exposed`; only intermediate T146/T148/T149/T150/T151/T159 may use `candidate`; T167 and later use `final`. Every behavior implementation task records failing command/result before implementation and same passing command after; decision/docs-only tasks record deterministic validation instead.

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
- every changed public contract receives same-commit documentation;
- run `git diff --check` before handoff;
- task subagents never commit or push; every authorized boundary stops for orchestrator under README ritual;
- WP-C3 T146–T147, WP-C4-C5A T148–T152, and WP-C7A T159–T160 are indivisible same-owner assignments; intermediate IDs remain `[~]`;
- public operation enters runtime catalog only in its exposure task.

## Phase 0 — Freeze implementation contracts

### T001 [x] Resolve runtime packaging and persistence direction
- **Depends:** none
- **Read:** `decisions.md` D001; foundation technology, architecture, invariants C1–C4.
- **Files:** `decisions.md`, `../../invariants.md`, `../../technology.md`, `../../architecture.md`, `../../testing.md`.
- **Deliver:** owner-approved native CLI/SQLite/gate/`xtask`/version/distribution policy plus exact crate-by-crate dependency/version/feature table for core, integrations, CLI, xtask, and standalone provider fixtures; root workspace explicitly excludes fixture crates; update technology statuses and resolution record.
- **Verify:** links/terminology checks and `git diff --check`.
- **Done when:** no dependent task must treat C1–C4 as candidate.
- **Stop:** owner rejects recommendation; re-plan every persistence/tooling task.

### T002 [x] Resolve supported platforms
- **Depends:** T001
- **Read:** D002; foundation trace, subprocess, distribution, and test isolation sections.
- **Files:** `decisions.md`, `../../technology.md`, `../../testing.md`.
- **Deliver:** exact MVP OS/architecture matrix and unsupported-platform policy published in `technology.md` and mirrored only where testing scope differs.
- **Verify:** `git diff --check` and consistency search for unresolved platform wording.
- **Done when:** process, permission, path, fixture, and CI tasks share one platform scope.
- **Stop:** Windows included; reopen D005, D007, D010, and D013 first.

### T003 [x] Select project license
- **Depends:** T001
- **Read:** D003 and dependency/release requirements.
- **Files:** `decisions.md`, `../../technology.md`, new `../../../LICENSE-MIT`, new `../../../LICENSE-APACHE`, new `../../../README.md` as selected by D003.
- **Deliver:** owner-selected canonical license file(s), root README license notice, and resolution record; T017 consumes decision into exact package metadata.
- **Verify:** exact canonical license text and `git diff --check`.
- **Done when:** dependency allowlist and package metadata can use one explicit policy.
- **Stop:** owner has not selected MIT or MIT/Apache-2.0.

### T004 [x] Freeze application operation catalog
- **Depends:** T001
- **Read:** D004; architecture operation list; all UX storyboards; testing closed-catalog rule.
- **Files:** `decisions.md`, new `../../operation-catalog.md`, `quality/facets/v1/{schema.json,README.md}`, `../../architecture.md`, `../../testing.md`, `../../ux-storyboards.md`.
- **Deliver:** stable IDs, exact command/argv/flag ownership, exact facet flags, D005 role/result-derived provider rows including every applicable evaluation error, stable reason-code taxonomy, facet-inventory v1 schema/path/status/evidence rules, normative lifecycle-family owner table matching `testing.md`, registration-wide `provider.check --active-runs` versus per-run compatibility ownership, update-without-approval versus disable-acknowledgement semantics, exact 21-ID architecture list, `run compatibility` storyboard example, mutation classification, explicit non-operations, and rationale for update/compatibility/export.
- **Verify:** every desired caller action and foundation outcome maps once; provider catalog success/rejection requires fresh catalog proof and no run journal; rejected creation has no run/journal; post-lookup run rejections require fresh history.
- **Done when:** exact catalog accepted and coverage table can be mechanical.
- **Stop:** any operation split/merge remains disputed.

### T005 [x] Freeze provider protocol v1 transport
- **Depends:** T001, T002
- **Read:** D005; provider model in intent/architecture/technology; I4, I6, I8, I9, I20, I32, I37–I40, I43–I44.
- **Files:** `decisions.md`, new `../../provider-protocol-v1.md`, `../../technology.md`, `../../architecture.md`.
- **Deliver:** process lifetime, stdin/stdout framing, version negotiation, five role names, role-specific result applicability matrix, inherited-environment/no-override/no-trace policy, resolve-once immutable config handoff, unknown-field policy, timeout/termination, and no-state-authority rule.
- **Verify:** protocol examples are language-neutral; each role maps only valid denial/finding/incompatibility/error variants; invoker cannot query catalog.
- **Done when:** independent provider author could implement transport without Rust source.
- **Stop:** persistent process, streaming protocol, or Windows support requested; re-design before schemas.

### T006 [x] Freeze structured CLI and exit contract
- **Depends:** T004
- **Read:** D006; I18, I27, I34, I46; UX automation storyboard.
- **Files:** `decisions.md`, new `../../cli-contract.md`, `../../ux-storyboards.md`, `../../testing.md`.
- **Deliver:** schema version, additive versus breaking compatibility rule/support duration, exact argv/flag surface, top-level fields, run summary, evidence status, reason format, stdout/stderr boundary, and exit codes.
- **Verify:** completed/rejected/error plus pre-dispatch examples validate manually against contract.
- **Done when:** renderer and E2E parser require no schema guesses.
- **Stop:** human and structured semantics diverge.

### T007 [x] Freeze machine-local paths and configuration
- **Depends:** T001, T002
- **Read:** D007; intent configuration; I16, I40–I41; technology persistence direction.
- **Files:** `decisions.md`, new `../../configuration.md`, `../../technology.md`, `../../architecture.md`.
- **Deliver:** exact config/state/trace paths, `LOOP_ENGINE_HOME`, TOML shape, precedence, unknown-key policy, symlink/lexical normalization, nonexistent executable/CWD behavior, configured-spelling identity rule, absolute registration paths, and malformed-config behavior.
- **Verify:** examples cover normal user home, isolated test home, symlink/nonexistent paths, unknown keys, and another caller CWD.
- **Done when:** project defaults cannot rebind an existing run.
- **Stop:** multiple state stores or provider definitions in project config proposed.

### T008 [x] Freeze bounds and timeout defaults
- **Depends:** T005, T006, T007
- **Read:** D008; evidence, provider, trace, and diagnostic bounds in foundation.
- **Files:** `decisions.md`, `../../provider-protocol-v1.md`, `../../configuration.md`, `../../technology.md`, `../../cli-contract.md`.
- **Deliver:** one named table for every payload/scalar/label/note/actor/path/argv/config/diagnostic/trace/timeout bound; count+byte keyset pagination for registration/run/evidence/history/active-compatibility/impact reports; ten-call provider-page ceiling and cross-process trace reservations; default/max count and page-byte budget; empty/malformed/wrong-version/filter-mismatched cursor behavior; CLI/schema owners.
- **Verify:** each bound/pagination rule has one source of truth and cursor-v1 schema/examples live in `cli-contract.md`; E2E owners are T147 registration/zero-impact list, T150/T152 run/history and active-compatibility pages, T157/T175 nonempty impact pages, T160 evidence list, and T101/T152/T182 trace-reservation limits.
- **Done when:** no implementation task needs a magic number.
- **Stop:** selected evidence would be truncated; foundation requires rejection instead.

### T009 [x] Freeze SQLite and migration policy
- **Depends:** T001, T007
- **Read:** D009; persistence authority; I12–I16, I35–I36, I45.
- **Files:** `decisions.md`, new `../../persistence.md`, `../../technology.md`, `../../architecture.md`, `../../testing.md`.
- **Deliver:** pragmas, busy policy, migration rules, schema-version compatibility, transaction boundaries, workflow and registration-config CAS semantics, atomic affected-run digest guard for catalog mutation, create-versus-update/disable/restore linearization, and no-write-lock provider rule.
- **Verify:** stale event/create attempts, both catalog/create writer orders, label/note/evidence concurrency, and rollback narratives are explicit.
- **Done when:** migration `0001` can be designed without semantic gaps.
- **Stop:** event sourcing, ORM, dual authority, or provider-spanning write transaction proposed.

### T010 [x] Freeze operational trace contract
- **Depends:** T002, T006–T008
- **Read:** D010; I42, I46; architecture/technology trace sections; UX trace contract.
- **Files:** `decisions.md`, new `../../operational-trace.md`, `../../technology.md`, `../../testing.md`, `../../ux-storyboards.md`.
- **Deliver:** JSONL v1 categories, permissions/startup/flush, encoded-byte actual-plus-reserved hard rotation budget, no raw/parsed duplication, concurrent coordination, help/version/parse behavior, late-sink semantics, and external `SIGXFSZ`/`RLIMIT_FSIZE` read-plus-mutation E2E contract.
- **Verify:** completed/rejected/error/parse/init/crash examples plus provider-read and committed-annotation `EFBIG` cases on supported Unix without production test branch.
- **Done when:** trace cannot become competing state/journal authority.
- **Stop:** design requires trace context in every function or claims impossible crash completeness.

### T011 [x] Freeze journal entry model
- **Depends:** T008–T009
- **Read:** D011; I8, I11–I15, I35; reference journal expectations.
- **Files:** `decisions.md`, new `../../journal-contract.md`, `../../architecture.md`, `../../ux-storyboards.md`.
- **Deliver:** entry kinds, 2.5 MiB aggregate/component bounds, attempt shape, required fields, sequence, correction link, provider/gate/evidence nesting, and non-replay statement.
- **Verify:** creation/mutation/rejection/provider/stale/guidance/correction examples plus maximum-size arithmetic and oversize rejection.
- **Done when:** persistence schema and history renderer share one immutable model.
- **Stop:** proposed model requires reconstructing current state by replay.

### T012 [x] Freeze semantic judge contract and provisioning
- **Depends:** T001
- **Read:** D012; I47; testing Git enforcement; architecture composition/enforcement.
- **Files:** `decisions.md`, new `../../development-policy.md`, `../../testing.md`, `../../architecture.md`.
- **Deliver:** generic executable request/result v1, foundation-seed rubric manifest for parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`, no-second-bootstrap rule, local/publication outcomes, timeout, citations, deterministic-evidence input, actual local judge executable/configuration, and named CI secret/config owner.
- **Verify:** real local publication-mode smoke against foundation parent plus pass/fail/indeterminate/unavailable fixture matrix; first post-foundation push remains blocked until T029.
- **Done when:** owner proves real judge runs locally and records exact T029 provisioning handoff without storing credentials.
- **Stop:** fixture/fake judge is proposed as publication authority.

### T013 [x] Freeze provider-fixture implementation strategy
- **Depends:** T002, T005
- **Read:** D013; no-mock doctrine; reference provider boundary.
- **Files:** `decisions.md`, `../../testing.md`, `../../technology.md`.
- **Deliver:** fixture language/runtime, package location, subprocess isolation, invocation ledger, and process-failure helper policy.
- **Verify:** fixture remains external executable and software concepts remain outside core.
- **Done when:** CI/runtime prerequisites are known.
- **Stop:** fixture imports product internals or accesses authoritative DB.

### T014 [x] Freeze canonical graph encoding
- **Depends:** T005, T008
- **Read:** D014; I6–I8, I32, I37, I43; graph validation direction.
- **Files:** `decisions.md`, `../../provider-protocol-v1.md`, new `../../graph-projection.md`, `../../technology.md`.
- **Deliver:** semantic ordering rules, included fields, exact canonical DTO version, metadata treatment, golden vectors, and SHA-256 identity distinction.
- **Verify:** field-change matrix states whether graph revision must change.
- **Done when:** no persisted run can later receive accidental digest redefinition.
- **Stop:** canonical form depends on hash-map iteration or provider raw JSON formatting.

### T015 [x] Freeze audit export scope
- **Depends:** T004, T006, T009, T011
- **Read:** D015; I24, I41, I45; persistence/export testing requirements.
- **Files:** `decisions.md`, new `../../export-contract.md`, `../../operation-catalog.md`, `../../testing.md`, `../../technology.md`.
- **Deliver:** `run.export` ownership, output directory behavior, state JSON/journal JSONL schemas, ordering, D006 additive/breaking compatibility and support-duration policy, and no-import guarantee.
- **Verify:** export cannot mutate, restore, replay, or dereference locators.
- **Done when:** optional foundation language and required test language agree.
- **Stop:** export to stdout would violate one-envelope structured mode without explicit design.

### T016 [x] Freeze project-default discovery and close C0
- **Depends:** T001–T015
- **Read:** D016; I40–I41; clean-room/no-discovery constraints.
- **Files:** `decisions.md`, `../../configuration.md`, `../../technology.md`.
- **Deliver:** ancestor-search algorithm, file name, boundary, allowed keys, precedence, and explicit distinction from provider discovery.
- **Verify:** examples prove no registration/executable rebinding.
- **Done when:** all D001–D016 rows and foundation updates pass; stop for orchestrator C0 commit `docs: freeze implementation contracts` with complete Phase 0 staged scope.
- **Stop:** project file becomes workflow/provider authoring source.

## Phase 1 — Workspace, governance, and architecture skeleton

### T017 [ ] Create reproducible empty Rust workspace shell
- **Depends:** T001–T016
- **Read:** I22–I24; architecture product crates; selected license/toolchain decisions.
- **Files:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.gitignore`.
- **Deliver:** Rust 2024 virtual workspace with empty member list, explicit `test-support/providers/*` exclusion, shared package version `0.1.0`, exact T001 toolchain/resolver, and no MSRV/installer claim; T018/T019 add members.
- **Verify:** `cargo metadata --format-version 1 --no-deps` reports zero packages; lockfile generation is reproducible.
- **Done when:** workspace shell parses from clean checkout without naming nonexistent members.
- **Stop:** any fourth product/runtime crate is introduced.

### T018 [ ] Create three product crate roots
- **Depends:** T017
- **Files:** `Cargo.toml`, `Cargo.lock`, `crates/loop-engine-core/{Cargo.toml,src/lib.rs}`, `crates/loop-engine-integrations/{Cargo.toml,src/lib.rs}`, `crates/loop-engine-cli/{Cargo.toml,src/main.rs}`.
- **Deliver:** add exactly three product members; preload complete T001-approved runtime/dev dependency versions/features into each manifest and lockfile so later product tasks need no manifest invention; add compile-only roots and inward dependency direction.
- **Verify:** `cargo check --workspace`; `cargo tree -p loop-engine-core` shows no outer dependency.
- **Done when:** CLI alone depends on both product libraries and is composition root placeholder.
- **Stop:** core receives Clap, Serde, SQLite, process, config, or tracing-subscriber dependency.

### T019 [ ] Create `xtask` command shell
- **Depends:** T017
- **Files:** `Cargo.toml`, `Cargo.lock`, `xtask/{Cargo.toml,src/main.rs,src/lib.rs}`.
- **Deliver:** add non-shipping workspace member with complete T001-approved xtask dependencies/lock update and command dispatcher with only implemented subcommands.
- **Verify:** `cargo run -p xtask -- help`.
- **Done when:** unknown command fails clearly and no product crate depends on `xtask`.
- **Stop:** gate logic duplicated in shell scripts.

### T020 [ ] Configure formatting, lint, and warnings policy
- **Depends:** T017–T019
- **Files:** `Cargo.toml`, `rustfmt.toml`, `../../development-policy.md`.
- **Deliver:** workspace formatting and Clippy warnings-denied commands with no broad allow list.
- **Verify:** `cargo fmt --all --check`; `cargo clippy --workspace --all-targets -- -D warnings`.
- **Done when:** canonical commands are documented once.
- **Stop:** lint suppression hides unfinished behavior.

### T021 [ ] Implement crate dependency architecture check
- **Depends:** T018–T019
- **Read:** architecture dependency direction; I22–I24.
- **Files:** `xtask/src/architecture.rs`, `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/{allowed,forbidden-edge}/*`.
- **Deliver:** Cargo-metadata check for exactly three product crates and allowed dependency edges.
- **Verify:** `cargo run -p xtask -- architecture`; deliberate forbidden-edge fixture fails.
- **Done when:** core outer dependency and integrations→CLI edge are mechanically rejected.
- **Stop:** check relies only on prose or current successful build.

### T022 [ ] Implement internal-boundary and bypass architecture checks
- **Depends:** T021
- **Read:** architecture model/capabilities/operations direction and choke-point rules.
- **Files:** `xtask/src/architecture.rs`, `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/{bypass,reversed-core,catch-all}/*`.
- **Deliver:** checks for model←capabilities←operations direction, sole composition root, approved provider/persistence/dispatch modules, and forbidden `util`/`common` catch-alls.
- **Verify:** canaries for direct process/SQLite construction and reversed core import fail.
- **Done when:** check enforces dependency/bypass boundaries without counting log syntax.
- **Stop:** custom compiler plugin or broad AST framework proposed.

### T023 [ ] Define semantic-judge v1 schemas
- **Depends:** T012, T019
- **Files:** `quality/semantic-judge/v1/{request,response}.schema.json`, `quality/semantic-judge/v1/fixtures/*.json`, `xtask/tests/semantic_judge_schema.rs`.
- **Deliver:** generic request and pass/fail/indeterminate response schemas with citations and deterministic evidence references.
- **Verify:** `cargo test -p xtask --test semantic_judge_schema`; valid/malformed request/response fixtures.
- **Done when:** schemas contain no Pi/model/provider-specific field.
- **Stop:** response permits uncited pass/fail or candidate rubric substitution.

### T024 [ ] Implement semantic-judge runner
- **Depends:** T019, T023
- **Files:** `xtask/src/semantic_judge.rs`, `xtask/tests/semantic_judge.rs`.
- **Deliver:** exact parent/candidate diff collection, parent rubric loading, executable invocation, timeout, schema validation, local/publication disposition.
- **Verify:** tests for pass, fail, indeterminate, unavailable, timeout, malformed response, consumed foundation bootstrap/seed-rubric fallback, rejection of second bootstrap, first unpublished range, and changed-rubric-next-commit behavior.
- **Done when:** publication mode fails closed and product runtime has no judge dependency.
- **Stop:** runner silently falls back to pass.

### T025 [ ] Author focused parent-versioned rubrics
- **Depends:** T012, T024
- **Files:** `quality/rubrics/{documentation,observability,architecture,behavioral-evidence}.md`, `quality/rubrics/manifest.json`, `quality/rubrics/fixtures/*.json`, `xtask/tests/semantic_judge.rs`.
- **Deliver:** narrow cited criteria matching foundation, including KISS and no invented test claims.
- **Verify:** `cargo test -p xtask --test semantic_judge`; manifest version/hash and cited example-decision fixtures.
- **Done when:** parent revision can supply complete rubric set deterministically.
- **Stop:** rubric changes foundation policy without owner decision.

### T026 [ ] Implement deterministic documentation checker
- **Depends:** T019
- **Files:** `xtask/src/docs_check.rs`, `xtask/tests/docs_check.rs`, `xtask/tests/fixtures/docs-check/*`.
- **Deliver:** UTF-8, final newline, trailing whitespace, relative-link, duplicate-heading, required-file, and terminology checks.
- **Verify:** focused invalid fixtures plus `cargo run -p xtask -- docs-check`.
- **Done when:** semantic judge is not asked to replace deterministic checks.
- **Stop:** checker attempts subjective semantic judgment.

### T027 [ ] Implement hook installer and pre-commit adapter
- **Depends:** T024–T026
- **Files:** `.githooks/pre-commit`, `xtask/src/hooks.rs`, `xtask/tests/hooks.rs`, `xtask/tests/fixtures/hooks/*`.
- **Deliver:** versioned thin hook; exact staged-tree deterministic checks and local semantic attempt; install/verify commands.
- **Verify:** staged pass, semantic fail, unavailable warning, unstaged contamination, and hook-version mismatch tests.
- **Done when:** hook delegates to canonical Rust logic and never rewrites user files.
- **Stop:** hook judges working tree instead of exact staged tree.

### T028 [ ] Implement exact-commit pre-push gate and incremental quality manifest
- **Depends:** T024–T027
- **Files:** `.githooks/pre-push`, `xtask/src/{publication,quality}.rs`, `quality/manifest.toml`, `xtask/tests/{publication,quality}.rs`, `xtask/tests/fixtures/publication/*`.
- **Deliver:** enumerate unpublished commits, detached worktree per commit, parent-rubric semantic pass, manifest-driven currently implemented canonical checks, fail-closed unavailable behavior; later tasks extend manifest and T195 freezes final set.
- **Verify:** multi-commit range tests including rubric change and failing middle commit.
- **Done when:** later good commit cannot repair/judge earlier bad commit.
- **Stop:** gate checks branch tip only.

### T029 [ ] Add authoritative CI adapter
- **Depends:** T028
- **Files:** `.github/workflows/quality.yml`, `../../development-policy.md`.
- **Deliver:** CI invokes same publication command, provisions real judge, records per-commit artifacts, and documents required branch rule.
- **Verify:** workflow syntax validation and local dry-run command documentation.
- **Done when:** no gate logic is duplicated in YAML.
- **Stop:** real judge cannot be provisioned; publication remains blocked.

### T030 [ ] Add dependency policy and close C1
- **Depends:** T003, T017–T029
- **Files:** `deny.toml`, `xtask/src/dependencies.rs`, `xtask/tests/dependencies.rs`, `../../development-policy.md`, `quality/evidence/v1/checkpoint.schema.json`.
- **Deliver:** allowed licenses/sources, advisory policy, lockfile checks, canonical command, and C1 closure covering workspace/governance/empty runtime surface.
- **Verify:** `cargo check --workspace --locked`; format/Clippy; `cargo run -p xtask -- architecture`; docs check; staged pre-commit path; real judge smoke; root help exposes zero application routes; `cargo deny check` or selected equivalent; fixture violations fail.
- **Done when:** C1 candidate is buildable, governed, architecture/dependency-policy-clean, exposes zero application operations, and stops for orchestrator commit `build: establish workspace governance`.
- **Stop:** required dependency conflicts with owner-selected license.

## Phase 2 — Core model and deterministic semantics

### T031 [ ] Establish core module boundaries
- **Depends:** T030
- **Files:** `crates/loop-engine-core/src/{lib.rs,model/mod.rs,capabilities/mod.rs,operations/mod.rs}`.
- **Deliver:** private-by-default model/capability/operation layout with documented dependency direction.
- **Verify:** `cargo check -p loop-engine-core`; architecture gate.
- **Done when:** no empty generic service/repository/util abstraction exists.
- **Stop:** an outer DTO is needed in core API; redesign boundary first.

### T032 [ ] Implement bounded scalar primitives
- **Depends:** T008, T031
- **Files:** `core/src/model/bounded.rs` (inline tests).
- **Deliver:** bounded text, diagnostic, metadata depth/count, and encoded-size validation without serialization annotations.
- **Verify:** boundary, Unicode-byte, depth, and oversize tests.
- **Done when:** all later model types reuse named bounds.
- **Stop:** truncation is used where foundation requires rejection.

### T033 [ ] Implement validated identifiers
- **Depends:** T008, T031–T032
- **Files:** `core/src/model/ids.rs`, tests.
- **Deliver:** distinct registration/run/request/state/event/gate/evidence/journal/graph-revision IDs and exact D004 lowercase-ASCII/no-normalization provider handle grammar.
- **Verify:** valid/empty/oversize/cross-type plus explicit 1-, 2-, 3-, and 128-character handle vectors, uppercase, Unicode, and invalid-character tests.
- **Done when:** IDs are not interchangeable accidentally and carry no external format derives.
- **Stop:** labels or executable paths become identity.

### T034 [ ] Implement time and internal version facts
- **Depends:** T031
- **Files:** `core/src/model/time.rs`, `version.rs`, tests.
- **Deliver:** timestamp value, workflow-state/lifecycle version, and journal sequence types.
- **Verify:** monotonic version/sequence transition tests.
- **Done when:** caller cannot construct or supply workflow revision token through public operations.
- **Stop:** label/note/evidence versions are coupled to gate CAS version.

### T035 [ ] Implement actor metadata and note model
- **Depends:** T032–T033
- **Files:** `core/src/model/annotation.rs`, tests.
- **Deliver:** bounded opaque actor metadata, notes, and correction link without authority methods.
- **Verify:** same decision inputs with different actor metadata remain equal in resolver tests later.
- **Done when:** no human/agent enum exists.
- **Stop:** actor field can satisfy gate or authorize transition.

### T036 [ ] Implement immutable input declarations and values
- **Depends:** T032–T033, T005, T008
- **Files:** `core/src/model/run_input.rs`, tests.
- **Deliver:** provider-declared required/optional names/descriptions and bounded accepted JSON-like values through core-owned value model.
- **Verify:** undeclared/missing/duplicate/oversize validation and no mutation API.
- **Done when:** input values cannot influence graph construction in core.
- **Stop:** generic mutable variable/update operation appears.

### T037 [ ] Implement append-only evidence model
- **Depends:** T008, T032–T035
- **Files:** `core/src/model/evidence.rs`, tests.
- **Deliver:** stable run-scoped ID, kind, exact bounded opaque locator, digest, media type, metadata, caller/provider source, and immutable record; no URI/path/CWD interpretation.
- **Verify:** empty/control/NUL/oversize rejection, opaque provider-relative convention preservation, association separation, same-locator distinct revisions, no dereference API.
- **Done when:** correction requires new evidence.
- **Stop:** engine interprets provider-specific artifact/workspace semantics.

### T038 [ ] Implement graph state and guidance model
- **Depends:** T032–T036
- **Files:** `core/src/model/graph.rs`, `guidance.rs`, tests.
- **Deliver:** flat states, initial ID, final flag, title/summary/metadata, static text or explicit no-guidance, and stored live-guidance capability.
- **Verify:** zero/one/multiple finals and initial-final representable.
- **Done when:** hierarchy/parallel/timer semantics are unrepresentable.
- **Stop:** candidate input values are accepted by graph constructor.

### T039 [ ] Implement transition and gate declarations
- **Depends:** T033, T038
- **Files:** `core/src/model/transition.rs`, tests.
- **Deliver:** source/event/target and unique required gate IDs supporting cycles/self-loops.
- **Verify:** cycle/self-loop/gate-free/multi-gate construction tests.
- **Done when:** provider cannot attach target-state authority to verdict.
- **Stop:** transition selection delegated to provider.

### T040 [ ] Implement semantic graph validation
- **Depends:** T038–T039
- **Files:** `core/src/model/graph_validation.rs`, tests.
- **Deliver:** initial/target checks, uniqueness, one `(state,event)`, final sinks, gate uniqueness, guidance declaration, supported semantics.
- **Verify:** complete valid/invalid matrix from testing doctrine.
- **Done when:** cycles, zero-final, multiple-final, initial-final, and non-final sink are valid.
- **Stop:** reachability/DAG policy rejects allowed graphs.

### T041 [ ] Implement canonical semantic projection model
- **Depends:** T014, T036, T038–T040
- **Files:** `core/src/model/graph_projection.rs`, `core/tests/graph_projection_golden.rs`.
- **Deliver:** core semantic view exposing all digest-relevant fields without Serde/canonical byte technology.
- **Verify:** reorder-equivalent graphs compare semantically; meaningful field changes differ.
- **Done when:** integration can encode canonical bytes without raw provider JSON.
- **Stop:** hash/canonical serializer leaks into core.

### T042 [ ] Implement provider identity and observation model
- **Depends:** T032–T034
- **Files:** `core/src/model/provider.rs`, tests.
- **Deliver:** immutable registration identity, mutable handle/config facts, locator/digest/version observations, and invocation phase facts.
- **Verify:** digest/path/version cannot compare as workflow identity.
- **Done when:** unavailable digest is representable without pretending equality.
- **Stop:** creation locator pins active run.

### T043 [ ] Implement gate result model
- **Depends:** T037, T039, T042
- **Files:** `core/src/model/gate.rs`, tests.
- **Deliver:** exact complete verdict set with pass/fail diagnostics/evidence, explicit incompatibility, or evaluation error.
- **Verify:** incompatible/error variants cannot carry evidence; duplicate/missing/extra verdict construction rejected.
- **Done when:** exactly three semantic result variants exist.
- **Stop:** provider result can choose target state.

### T044 [ ] Implement compatibility and live-guidance model
- **Depends:** T037–T043
- **Files:** `core/src/model/compatibility.rs`, `live_guidance.rs`, tests.
- **Deliver:** capability-scoped non-latching findings and `LiveGuidanceResult` variants for bounded advisory guidance, explicit stored-guidance incompatibility, or evaluation error, all without evidence/state authority.
- **Verify:** mixed support findings, all three guidance results, and no evidence-bearing guidance.
- **Done when:** compatibility state cannot be persisted as latch through model API.
- **Stop:** registration-wide report mutates every run.

### T045 [ ] Implement run aggregate and lifecycle
- **Depends:** T033–T044
- **Files:** `core/src/model/run.rs`, `lifecycle.rs`, tests.
- **Deliver:** stable ID, registration ID, graph snapshot/revision, immutable inputs, current state, active/final/terminated lifecycle, workflow version, optional non-unique label.
- **Verify:** initial-final, active-only label, terminal no-reopen, terminal annotation allowance.
- **Done when:** current state is direct authority and no replay constructor exists.
- **Stop:** run identity depends on workspace/provider path/label.

### T046 [ ] Implement immutable journal facts
- **Depends:** T011, T034–T045
- **Files:** `core/src/model/journal.rs`, `attempt.rs`, tests.
- **Deliver:** bounded aggregate entry kinds and required observed facts, bounded association/provider/verdict/diagnostic components, state/version before/after, outcome, correction link; encoded maximum 2.5 MiB.
- **Verify:** contract examples and maximum-component arithmetic construct below aggregate bound; one-byte-over rejects; edit/delete/fold APIs absent.
- **Done when:** journal explains but cannot claim replay.
- **Stop:** current run state is derived from entries.

### T047 [ ] Implement public outcome and reason taxonomy
- **Depends:** T006, T032–T046
- **Files:** `core/src/model/outcome.rs`, `reason.rs`, `diagnostic.rs`, tests.
- **Deliver:** implement D004/D006 frozen completed/rejected/error and reason-code catalog, run snapshot, state-changed flag, requestable events, and evidence-recorded status; no new code may originate here.
- **Verify:** every frozen reason maps exactly once; rejected creation maps to no-run/no-journal; self-loop completes unchanged.
- **Done when:** no fourth top-level class exists.
- **Stop:** persistence failure reports evidence/state committed without proof.

### T048 [ ] Implement gate-free deterministic transition resolver
- **Depends:** T039–T047
- **Files:** `core/src/model/decision.rs`, tests.
- **Deliver:** active lifecycle/event resolution, unknown/ambiguous rejection, target/final lifecycle, self-loop, requestable-event calculation, and no-provider result.
- **Verify:** table tests for linear/cycle/self-loop/finals/sinks/terminal/unknown.
- **Done when:** identical inputs yield identical decision regardless of actor metadata.
- **Stop:** resolver invokes external capability.

### T049 [ ] Implement gated deterministic decision resolver
- **Depends:** T043, T047–T048
- **Files:** `core/src/model/decision.rs`, tests.
- **Deliver:** exact verdict enforcement, fail/incompatibility/evaluation classifications, state preservation, provider evidence handling, and target authority from stored graph only.
- **Verify:** pass/fail/mixed/incompatible/error/malformed-set matrix.
- **Done when:** only complete all-pass result advances.
- **Stop:** provider-reported target or substituted gate accepted.

### T050 [ ] Implement requestable-event projection
- **Depends:** T038–T049
- **Files:** `core/src/model/requestable.rs`, tests.
- **Deliver:** current-state event/gate summaries for active run; empty set for terminal; no gate-pass prediction.
- **Verify:** zero-event sink, multi-event, self-loop, terminal cases.
- **Done when:** show/outcomes can share one provider-free projection.
- **Stop:** computation executes provider or compatibility check.

### T051 [ ] Add pure core property tests
- **Depends:** T040, T048–T050
- **Files:** `core/tests/model_properties.rs`.
- **Deliver:** no-mock generated graphs/decisions for determinism, rejection preservation, final sinks, and actor neutrality.
- **Verify:** deterministic seed replay command.
- **Done when:** tests supplement rather than claim behavioral authority.
- **Stop:** property model imports outer integrations or replaces E2E commitments.

### T052 [ ] Document core model and decision boundaries
- **Depends:** T031–T051
- **Files:** `core/src/lib.rs`, `../../architecture.md`, `../../graph-projection.md`, `../../journal-contract.md`.
- **Deliver:** exact model invariants, deterministic inputs, excluded semantics, and no-replay/no-provider-authority statements.
- **Verify:** `cargo doc --workspace --no-deps`; docs check.
- **Done when:** core behavior introduced so far is same-commit coherent.
- **Stop:** docs claim public operations before exposure tasks.

## Phase 3 — Core capabilities and private operations

No capability mocks, fakes, or in-memory port implementations are permitted. Private-operation focused tests cover pure validation/decision functions, command construction, and compile-time capability contracts only. Durable effects and orchestration claims remain explicitly open until production integrations and black-box exposure tasks; Phase 3 checks never count as behavioral acceptance.

### T053 [ ] Define time, ID, and digest capabilities
- **Depends:** T031–T052
- **Files:** `core/src/capabilities/{time,id_generator,digest}.rs`.
- **Deliver:** narrow external-effect contracts using core types only.
- **Verify:** architecture gate and compile-only contract tests.
- **Done when:** operations need no system clock/UUID/hash dependency.
- **Stop:** generic dependency-injection container proposed.

### T054 [ ] Define targeted decision-event capability
- **Depends:** T010, T047–T050
- **Files:** `core/src/capabilities/decision_events.rs`.
- **Deliver:** narrow consequential decision events for transition/gate/lifecycle/compatibility/stale facts, not raw JSON logging.
- **Verify:** architecture compile test; event variants map to trace contract.
- **Done when:** pure helpers require no logging parameter.
- **Stop:** trace context threaded through every model function.

### T055 [ ] Define provider catalog capability
- **Depends:** T008, T042, T045, T053
- **Files:** `core/src/capabilities/provider_catalog.rs`.
- **Deliver:** add/list/resolve/update/rename/disable/restore, immutable `ResolvedProviderConfig` carrying config revision, byte/count-paged active-run impact/snapshot queries, active-set digest, and atomic guarded catalog-mutation commands.
- **Verify:** contract expresses immutable ID, enabled-handle uniqueness, tombstone restore, explicit config, one resolution per provider-dependent operation, and no query-then-mutate TOCTOU API.
- **Done when:** run operations resolve by stored registration ID.
- **Stop:** handle/path/digest becomes identity.

### T056 [ ] Define five-role provider invocation capability
- **Depends:** T005, T036–T045, T053
- **Files:** `core/src/capabilities/provider_invoker.rs`.
- **Deliver:** typed five-role calls accepting immutable resolved config plus bounded snapshots/observations and explicit pre-launch trace-budget-unavailable error.
- **Verify:** invoker exposes no catalog lookup, target-state setter, caller CWD, shell string, retry, environment override, or unbounded payload field; budget error proves provider did not launch.
- **Done when:** all role results preserve three semantic gate variants.
- **Stop:** provider DTO/Serde types cross inward.

### T057 [ ] Define provider-free run read capability
- **Depends:** T008, T045–T047
- **Files:** `core/src/capabilities/run_reader.rs`.
- **Deliver:** get/list/show graph/evidence/history and selected-evidence snapshot reads from authoritative state.
- **Verify:** no provider or journal-replay dependency in signatures.
- **Done when:** active-default and terminal-inclusive filters are representable.
- **Stop:** labels accepted as unique run lookup.

### T058 [ ] Define atomic run mutation capability
- **Depends:** T045–T047, T053
- **Files:** `core/src/capabilities/run_writer.rs`, `persistence_commands.rs`.
- **Deliver:** atomic create, evidence, annotation, label, termination, guidance-attempt, and per-run compatibility-attempt commands with journal facts.
- **Verify:** each command requires complete authority+journal payload and exposes no partial write.
- **Done when:** state mutation cannot bypass matching journal request.
- **Stop:** generic save/update method appears.

### T059 [ ] Define atomic event-attempt capability
- **Depends:** T046–T049, T057–T058
- **Files:** `core/src/capabilities/event_attempt_writer.rs`.
- **Deliver:** one command for completed/rejected/error attempts with inline/selected/provider evidence, expected workflow version, state transition, and committed-status result.
- **Verify:** stale branch can append error attempt without applying transition; persistence failure cannot claim recording.
- **Done when:** provider invocation needs no held write lock.
- **Stop:** label/note/evidence mutation invalidates expected workflow version.

### T060 [ ] Define read-only export capability
- **Depends:** T015, T046, T057
- **Files:** `core/src/capabilities/audit_export.rs`.
- **Deliver:** consistent read snapshot contract and explicit output target facts without import/restore API.
- **Verify:** export operation cannot mutate run/store.
- **Done when:** state/journal/evidence/provider observations are representable.
- **Stop:** JSON DTO annotations enter core.

### T061 [ ] Add capability architecture contract tests
- **Depends:** T053–T060
- **Files:** `xtask/tests/architecture.rs`, `xtask/tests/fixtures/architecture/capabilities/*`.
- **Deliver:** compile/dependency checks for core-only types, no integration construction, and operation→capability→model direction.
- **Verify:** `cargo run -p xtask -- architecture` with canaries.
- **Done when:** accidental outer leakage fails before behavior tests.
- **Stop:** tests depend on source-text logging counts.

### T062 [ ] Implement runtime operation catalog and baseline closure tool
- **Depends:** T004, T019, T031, T047
- **Files:** `core/src/operations/catalog.rs`, `xtask/src/operation_coverage.rs`, tests.
- **Deliver:** stable `OperationId`, iteration, uniqueness, facet metadata link, initially empty exposed set, and baseline extensible closure command with core collector plus named collector-registration API and empty-set support. Runtime collectors enumerate only currently registered/exposed IDs; D004's 21-ID table remains target documentation until each exposure and final T167 assertion.
- **Verify:** `cargo run -p xtask -- operation-coverage --mode baseline`; core-only empty-set pass plus duplicate/missing-core canaries; driver/routes are added by T133, E2E/trace by T145.
- **Done when:** operation can be added only through one reviewed API and planned/unexposed IDs cannot appear in any runtime collector.
- **Stop:** planned-but-unimplemented IDs appear in runtime exposed set.

### T063 [ ] Implement private `provider.add` operation
- **Depends:** T047, T053, T055
- **Files:** `core/src/operations/provider_add.rs` (inline tests).
- **Deliver:** validate explicit config/handle, allocate immutable ID, persist through catalog capability.
- **Verify:** pure handle/config validation and `DuplicateHandle` capability-error mapping; durable duplicate/distinct-ID proof remains T146–T147.
- **Done when:** operation remains unexposed until T147.

### T064 [ ] Implement private `provider.list` operation
- **Depends:** T008, T047, T055
- **Files:** `core/src/operations/provider_list.rs`, tests.
- **Deliver:** provider-free enabled/tombstoned registration listing plus `--active-runs-for` impact listing under D008 count/byte cursor contract.
- **Verify:** pure filter/cursor validation, page construction, and query/result mapping cover malformed/version/filter/count/byte/no-truncation branches; durable registration/impact paging remains T147/T157/T175.
- **Done when:** operation remains unexposed until T147.

### T065 [ ] Implement private `provider.check` operation
- **Depends:** T040–T044, T047, T055–T057
- **Files:** `core/src/operations/provider_check.rs`, tests.
- **Deliver:** explicit protocol/conformance/latest-graph check plus count/byte paging with one `describe` plus at most nine compatibility calls (ten total); resolve config once per page; stable keyset iteration; zero-page completion; pre-launch trace-budget error before first row errors unchanged cursor, after progress returns cursor; other failures error whole page; no truncated finding, journal fan-out, or latch.
- **Verify:** pure zero/mixed/incompatible/result-error aggregation and command construction; production catalog-resolution count and no-journal proof remain T152 provider-ledger E2Es.
- **Done when:** no run is created and operation remains unexposed until atomic T152 closure.

### T066 [ ] Implement private `provider.update` operation
- **Depends:** T047, T055
- **Files:** `core/src/operations/provider_update.rs`, tests.
- **Deliver:** atomically replace executable/argv/CWD/timeout under same ID, increment config revision, and return affected count plus `provider.list --active-runs-for` cursor/link without approval flag or unbounded IDs.
- **Verify:** pure command/result mapping preserves ID/config revision; durable linearization remains T107–T119/T178 and current-config use remains T163/T175.
- **Done when:** operation remains unexposed until T155.

### T067 [ ] Implement private `provider.rename` operation
- **Depends:** T047, T055
- **Files:** `core/src/operations/provider_rename.rs`, tests.
- **Deliver:** active registration handle rename preserving ID.
- **Verify:** pure handle validation and capability command/error mapping; durable uniqueness and run-binding stability remain T156/T175.
- **Done when:** operation remains unexposed until T156.

### T068 [ ] Implement private `provider.disable` operation
- **Depends:** T045, T047, T055
- **Files:** `core/src/operations/provider_disable.rs`, tests.
- **Deliver:** non-mutating warning-page commands where only final page yields opaque full-set/config-bound token, plus atomic `--allow-active-runs <ack-token>` tombstone command preserving ID/revision rules.
- **Verify:** pure first/intermediate cursor cannot authorize; final token maps to guarded command with no query-then-write API; durable proof remains T107–T119/T157/T178.
- **Done when:** operation remains unexposed until T157.

### T069 [ ] Implement private `provider.restore` operation
- **Depends:** T068
- **Files:** `core/src/operations/provider_restore.rs`, tests.
- **Deliver:** restore exact tombstoned ID with explicit config and free handle.
- **Verify:** pure restore-command/error mapping preserves ID and classifies occupied handle; durable restore/rejection proof remains T158/T175.
- **Done when:** operation remains unexposed until T158.

### T070 [ ] Implement private `run.create` operation
- **Depends:** T040–T047, T053, T055–T059
- **Files:** `core/src/operations/run_create.rs`, tests.
- **Deliver:** resolve once, describe, validate graph, conditionally validate inputs, detect observed digest drift, compute revision, initialize lifecycle, atomically store creation journal.
- **Verify:** pure zero-input skip, input/graph/drift classification, and initial-lifecycle command construction; durable no-run/journal proof remains T149/T152.
- **Done when:** operation remains unexposed until T152.

### T071 [ ] Implement private `run.list` operation
- **Depends:** T008, T045, T047, T057
- **Files:** `core/src/operations/run_list.rs`, tests.
- **Deliver:** provider-free active-default and explicit terminal/all keyset-paged listing under D008 cursor contract.
- **Verify:** pure filter/cursor parsing and query-command construction prove no CWD/provider field and no label identity; durable paging remains T150/T152.
- **Done when:** operation remains unexposed until T152.

### T072 [ ] Implement private `run.show` operation
- **Depends:** T045, T047, T050, T057
- **Files:** `core/src/operations/run_show.rs`, tests.
- **Deliver:** lifecycle/state/inputs/static guidance/gates/live capability/empty selection/requestable events.
- **Verify:** pure lifecycle projection empties terminal events and command shape has no provider capability; durable read/no-invocation proof remains T153.
- **Done when:** operation remains unexposed until T153.

### T073 [ ] Implement private `run.graph` operation
- **Depends:** T041, T045, T047, T057
- **Files:** `core/src/operations/run_graph.rs`, tests.
- **Deliver:** provider-free complete stored projection and graph revision.
- **Verify:** pure projection accepts stored snapshot only; durable drift/missing-provider proof remains T154.
- **Done when:** operation remains unexposed until T154.

### T074 [ ] Implement private `run.history` operation
- **Depends:** T008, T046–T047, T057
- **Files:** `core/src/operations/run_history.rs`, tests.
- **Deliver:** ordered provider-free journal query with D008 sequence-keyset pagination/filter contract.
- **Verify:** pure cursor/filter validation and ordered-row projection contain no state reconstruction/provider call; durable history paging remains T152.
- **Done when:** operation remains unexposed until T152.

### T075 [ ] Implement private `run.evidence.add` operation
- **Depends:** T037, T046–T047, T053, T058
- **Files:** `core/src/operations/evidence_add.rs`, tests.
- **Deliver:** validate/ID/append evidence and journal atomically on any lifecycle.
- **Verify:** pure locator validation, ID-bearing append-command construction, and lifecycle classification; durable duplicate-locator/terminal atomic append remains T159–T160.
- **Done when:** operation remains unexposed until T160.

### T076 [ ] Implement private `run.evidence.list` operation
- **Depends:** T008, T037, T047, T057
- **Files:** `core/src/operations/evidence_list.rs`, tests.
- **Deliver:** provider-free stable keyset-paged inventory with prior event associations under D008 cursor contract.
- **Verify:** pure cursor/filter validation and evidence-row projection are provider-free; durable terminal/missing-provider paging remains T160.
- **Done when:** operation remains unexposed until T160.

### T077 [ ] Implement private `run.annotate` operation
- **Depends:** T035, T046–T047, T058
- **Files:** `core/src/operations/run_annotate.rs`, tests.
- **Deliver:** append note/opaque actor/correction journal fact without authority.
- **Verify:** pure note/actor/correction validation and append-command construction preserve state/version fields; durable lifecycle/history proof remains T161.
- **Done when:** operation remains unexposed until T161.

### T078 [ ] Implement private `run.label` operation
- **Depends:** T045–T047, T058
- **Files:** `core/src/operations/run_label.rs`, tests.
- **Deliver:** active-only optional non-unique label replacement plus journal.
- **Verify:** pure lifecycle classification and label-command construction preserve run identity/workflow version; durable terminal rejection and rebinding proof remain T162.
- **Done when:** operation remains unexposed until T162.

### T079 [ ] Implement private `run.request` orchestration
- **Depends:** T048–T050, T054–T059
- **Files:** `core/src/operations/run_request.rs` (inline tests).
- **Deliver:** run lookup, selection validation, event resolution, gate-free path, batched gated path, decision, CAS attempt commit, accurate evidence status, no retry.
- **Verify:** pure selection/evidence/verdict/stale classification and completed/rejected/error command construction only; durable lookup, CAS, persistence, invocation, and attempt-outcome proof remains T112–T113/T163.
- **Done when:** operation remains unexposed until T163 and no write lock contract spans provider.
- **Stop:** full applicable facets cannot be enumerated in coverage map.

### T080 [ ] Implement private `run.guidance` operation
- **Depends:** T044–T047, T054–T059
- **Files:** `core/src/operations/run_guidance.rs`, tests.
- **Deliver:** active/stored-capability checks, explicit provider call, advisory/incompatibility/evaluation result mapping, completed/rejected/error journaling, no evidence/state mutation.
- **Verify:** pure stored-unsupported/incompatibility/terminal/error classification and journal-command construction; no-invocation and post-lookup persistence proof remain T164 production E2Es.
- **Done when:** operation remains unexposed until T164.

### T081 [ ] Implement private `run.compatibility` operation
- **Depends:** T044–T047, T054–T058
- **Files:** `core/src/operations/run_compatibility.rs`, tests.
- **Deliver:** active-run explicit current-provider check with mixed non-latching findings and compatibility-attempt journal command carrying actual provider locator/digest/version and observed drift, without state/latch mutation.
- **Verify:** pure finding/lifecycle/drift classification and journal-command construction; durable atomic invocation/history proof remains T114/T165.
- **Done when:** operation remains unexposed until T165.

### T082 [ ] Implement private `run.terminate` operation
- **Depends:** T045–T047, T058
- **Files:** `core/src/operations/run_terminate.rs`, tests.
- **Deliver:** active→terminated mutation with optional note and journal, no provider.
- **Verify:** pure lifecycle classification and versioned termination-command construction; durable repeat/final rejection, no-reopen, increment, and journal proof remains T151–T152.
- **Done when:** operation remains unexposed until T152.

### T083 [ ] Implement private `run.export` operation
- **Depends:** T015, T047, T060
- **Files:** `core/src/operations/run_export.rs`, tests.
- **Deliver:** consistent read-only audit snapshot request to explicit target.
- **Verify:** pure request/command shape exposes no import/mutation/replay/dereference path; durable export safety remains T166/T181.
- **Done when:** operation remains unexposed until T166.

## Phase 4 — Provider, configuration, trace, and system integrations

### T084 [ ] Define provider protocol and published schemas
- **Depends:** T005, T008, T014, T017, T031–T050
- **Files:** `integrations/src/provider_protocol/{mod,dto,version}.rs`, `schemas/provider/v1/*.json`.
- **Deliver:** request/result envelopes and five operation schemas with bounds and same-major compatibility.
- **Verify:** schema golden fixtures for every valid result variant and malformed/unsupported cases.
- **Done when:** schema contains no state-setter and input validation cannot emit topology.

### T085 [ ] Implement common provider DTO mapping
- **Depends:** T084
- **Files:** `integrations/src/provider_protocol/mapping.rs`, `validation.rs`, tests.
- **Deliver:** external DTO↔core conversion with structural bounds and actionable paths.
- **Verify:** unknown optional fields accepted; unknown required semantics, oversize, wrong tags, and malformed metadata fail.
- **Done when:** Serde/Schemars types remain outside core.

### T086 [ ] Implement graph mapping, semantic validation, and canonical bytes
- **Depends:** T014, T040–T041, T084–T085
- **Files:** `integrations/src/provider_protocol/{graph,canonical}.rs`, `integrations/tests/graph_canonical.rs`, `integrations/tests/fixtures/graphs/*.json`.
- **Deliver:** DTO→validated core graph→canonical DTO/bytes→SHA-256 input.
- **Verify:** reordering equivalence and every digest-relevant field-change vector.
- **Done when:** raw provider JSON formatting cannot affect graph revision.

### T087 [ ] Implement subprocess spawn and lifecycle
- **Depends:** T002, T005, T007–T008, T056, T084
- **Files:** `integrations/src/provider_process/{mod,spawn,error}.rs`, tests.
- **Deliver:** literal executable/argv, explicit CWD, inherited caller environment with no registration overrides, stdin request, closed EOF, status/signal classification, no shell.
- **Verify:** caller-CWD independence, provider sees inherited test sentinel, missing executable, nonzero, signal, and no environment field/value appears in trace payload.
- **Done when:** arbitrary provider output never reaches CLI streams directly.

### T088 [ ] Implement deadlock-safe bounded stream capture
- **Depends:** T087
- **Files:** `integrations/src/provider_process/streams.rs`, tests.
- **Deliver:** concurrent stdout/stderr drain, byte bounds, UTF-8/error policy, exact one-result extraction.
- **Verify:** simultaneous full streams, oversize stdout/stderr, missing result, extra stdout, invalid UTF-8.
- **Done when:** provider cannot deadlock on full pipe.

### T089 [ ] Implement provider timeout and process-group termination
- **Depends:** T002, T087–T088
- **Files:** `integrations/src/provider_process/{timeout,process_group}.rs`, `integrations/tests/provider_timeout.rs`.
- **Deliver:** configurable deadline, process-group kill, child cleanup, timeout observation, no retry.
- **Verify:** real blocking provider and descendant process terminate within test budget.
- **Done when:** no orphan survives supported-platform timeout.
- **Stop:** platform cannot support selected semantics; reopen D002/D005.

### T090 [ ] Implement executable digest observation
- **Depends:** T053, T087
- **Files:** `integrations/src/provider_process/digest.rs`, tests.
- **Deliver:** best-effort pre-invocation SHA-256 and normalized actual locator; unavailable digest is explicit.
- **Verify:** readable, unreadable, replaced, and interpreted-script cases.
- **Done when:** digest is audit fact, never identity or environment proof.

### T091 [ ] Implement provider `describe` adapter
- **Depends:** T085–T090
- **Files:** `integrations/src/provider_protocol/describe.rs`, tests.
- **Deliver:** input-free request, graph/declarations/guidance mapping, observation capture.
- **Verify:** candidate inputs absent; malformed/invalid result classified.
- **Done when:** complete graph can be validated and canonicalized.

### T092 [ ] Implement provider input-validation adapter
- **Depends:** T036, T085–T090
- **Files:** `integrations/src/provider_protocol/validate_inputs.rs`, tests.
- **Deliver:** value-only request and accepted/rejected/error mapping with topology-output rejection.
- **Verify:** zero-input caller path can skip adapter; invalid values remain domain rejection.
- **Done when:** description/validation digest observations can be compared.

### T093 [ ] Implement provider gate-evaluation adapter
- **Depends:** T037, T043, T085–T090
- **Files:** `integrations/src/provider_protocol/evaluate_gates.rs`, tests.
- **Deliver:** bounded snapshot, complete requested gate set, three result variants, provider evidence validation.
- **Verify:** missing/extra/duplicate/substituted verdict, invalid evidence, incompatibility-with-evidence, evaluation-error-with-evidence.
- **Done when:** only complete verdict result can carry valid evidence.

### T094 [ ] Implement provider live-guidance adapter
- **Depends:** T044, T085–T090
- **Files:** `integrations/src/provider_protocol/live_guidance.rs`, tests.
- **Deliver:** bounded active-run context and exactly one result: advisory guidance, explicit stored-guidance incompatibility, or evaluation error.
- **Verify:** incompatibility maps distinctly; evidence/state fields rejected; guidance bound enforced.
- **Done when:** adapter cannot authorize transition.

### T095 [ ] Implement provider compatibility adapter
- **Depends:** T044, T085–T090
- **Files:** `integrations/src/provider_protocol/compatibility.rs`, tests.
- **Deliver:** stored graph/declaration capability request and mixed finding mapping.
- **Verify:** non-latching support/incompatibility/error cases.
- **Done when:** latest graph is not substituted for stored run graph.

### T096 [ ] Implement machine-local path resolver
- **Depends:** T002, T007, T030
- **Files:** `integrations/src/configuration/paths.rs`, tests.
- **Deliver:** OS roots, `LOOP_ENGINE_HOME`, DB/trace/global/project locations, ancestor search.
- **Verify:** isolated home, root boundary, symlink policy, another CWD, no project file.
- **Done when:** paths are deterministic and provider discovery never occurs.

### T097 [ ] Implement typed TOML configuration DTOs
- **Depends:** T007–T008, T096
- **Files:** `integrations/src/configuration/{dto,load,error}.rs`, tests.
- **Deliver:** global/project defaults parsing, unknown-key policy, malformed diagnostics, no registration definitions in project file.
- **Verify:** valid/malformed/unsupported/forbidden-key fixtures.
- **Done when:** external config DTOs do not leak inward.

### T098 [ ] Implement configuration precedence and registration defaults
- **Depends:** T097
- **Files:** `integrations/src/configuration/resolved.rs`, tests.
- **Deliver:** CLI > project > global > built-in merge for allowed defaults/references.
- **Verify:** full precedence matrix and existing-run stored-ID bypass of defaults.
- **Done when:** caller CWD cannot alter active-run provider config.

### T099 [ ] Implement trace event DTO schema
- **Depends:** T010, T023, T047, T054
- **Files:** `integrations/src/trace/{event,schema}.rs`, `schemas/trace/v1/*.json`, `schemas/trace/v1/fixtures/*.json`.
- **Deliver:** versioned dispatcher/provider/persistence/decision event DTOs and payload bound markers.
- **Verify:** schema fixtures for start/result/error/rollback/stale/crash plus exact on-disk maximum encoding for control-heavy JSON and binary/base64 streams; raw/parsed duplication canary fails.
- **Done when:** events remain diagnostics, not replay/journal records.

### T100 [ ] Implement secure per-invocation trace writer
- **Depends:** T002, T007–T008, T099
- **Files:** `integrations/src/trace/{mod,writer,error}.rs`, tests.
- **Deliver:** unique file, current-user-only creation, request ID, buffered/flush policy, init failure.
- **Verify:** Unix mode tests, collisions, concurrent files, write/flush failures.
- **Done when:** init failure is distinguishable before dispatch.

### T101 [ ] Implement cross-process trace rotation
- **Depends:** T100
- **Files:** `integrations/src/trace/rotation.rs`, tests.
- **Deliver:** 128 MiB cap over encoded actual plus unused reservation remainder, atomic write conversion from reserved→actual, 16 MiB base/10 MiB call additions, cross-process coordination, deterministic closed victim, 120 MiB active cap.
- **Verify:** maximum ten-call arithmetic never double-counts; reservation consume/exhaust/release and concurrent writers never exceed 128 MiB; active preservation/stale-lock recovery.
- **Done when:** hard directory bound and complete launched-call trace coexist under concurrency.

### T102 [ ] Implement traced provider boundary wrapper
- **Depends:** T087–T101
- **Files:** `integrations/src/provider_process/traced.rs`, tests.
- **Deliver:** acquire 10 MiB encoded-call reservation before launch; embed request JSON once, base64 each raw stream once, avoid parsed-result duplication, record facts/timing/correlation, consume reservation as writes land, release only unused remainder at trace close.
- **Verify:** maximum encoded JSON/control/binary fixtures fit reservation; one-byte-over cannot launch; all failures emit last observable event; inherited environment absent.
- **Done when:** no alternate production provider path bypasses wrapper.

### T103 [ ] Implement system clock, ID, and SHA-256 adapters
- **Depends:** T053, T017
- **Files:** `integrations/src/{system_clock,uuid_ids,sha256_digest}.rs`, tests.
- **Deliver:** UTC timestamps, selected IDs, graph/executable digest implementation.
- **Verify:** format, concurrency uniqueness, known SHA vectors.
- **Done when:** graph and executable digest types remain distinct.

## Phase 5 — SQLite authority, journal, and export

### T104 [ ] Implement SQLite open and migration runner
- **Depends:** T001, T007, T009, T030, T096
- **Files:** `integrations/src/persistence/{mod,sqlite,migrations,error}.rs`.
- **Deliver:** bundled open, pragmas, busy policy, transactional ordered migrations, future-schema refusal.
- **Verify:** empty/latest/concurrent open, failed migration rollback, unsupported future version.
- **Done when:** migration execution is traced-capable but independent of CLI.

### T105 [ ] Design and freeze migration `0001`
- **Depends:** T009, T011, T014, T036–T046, T055–T060, T104
- **Files:** `integrations/migrations/0001_initial.sql`, `../../persistence.md`, `../../persistence-schema.md`.
- **Deliver:** registrations with config revision, runs with resolved registration revision, snapshots/inputs, evidence, associations, journal, sequences, workflow versions, constraints, indexes.
- **Verify:** schema review against every atomic/stale/terminal/tombstone invariant and SQL integrity tests.
- **Done when:** query authority is direct and no run-delete path exists.
- **Stop:** stale attempt/evidence association cannot be represented atomically.

### T106 [ ] Implement persistence record DTO mappings
- **Depends:** T041–T047, T105
- **Files:** `integrations/src/persistence/{records,mapping}.rs`, tests.
- **Deliver:** integration-owned versioned JSON/row DTOs and corruption-validating core mappings.
- **Verify:** round trips plus unknown version/enum, malformed JSON, graph digest mismatch, invalid state/lifecycle.
- **Done when:** core has no Rusqlite/Serde annotations.

### T107 [ ] Implement provider catalog persistence
- **Depends:** T055, T104–T106
- **Files:** `integrations/src/persistence/provider_catalog.rs`, tests.
- **Deliver:** add/list/resolve plus config revision, count/byte-paged impact, final-page acknowledgement token binding, atomic guarded update/disable/restore, and handle uniqueness.
- **Verify:** concurrent handle claims; both run-create/catalog-mutation writer orders; tombstone release; exact-ID restore; handle reuse without rebinding.
- **Done when:** registration ID remains immutable and referenced tombstone retained.

### T108 [ ] Implement atomic run-creation transaction
- **Depends:** T058, T103–T107
- **Files:** `integrations/src/persistence/run_create.rs`, tests.
- **Deliver:** atomically recheck enabled registration/config revision then commit run/snapshot/inputs/state/version/registration/creation journal.
- **Verify:** initial-final; update/disable/restore between resolve and commit yields stale-provider-config error/no run; injected failure at each write boundary leaves all-or-nothing state.
- **Done when:** rejected/error/stale-config creation writes no run journal.

### T109 [ ] Implement provider-free run read queries
- **Depends:** T057, T105–T106
- **Files:** `integrations/src/persistence/run_reads.rs`, tests.
- **Deliver:** get/list/show/graph with active/terminal filters and authoritative columns.
- **Verify:** fresh connection restart, caller CWD irrelevance, no journal replay.
- **Done when:** missing provider has no effect on reads.

### T110 [ ] Implement evidence inventory and selected-context reads
- **Depends:** T037, T057, T105–T106
- **Files:** `integrations/src/persistence/evidence_reads.rs`, tests.
- **Deliver:** ordered inventory, associations, exact ID selection, complete context bound check.
- **Verify:** missing ID, unrelated history exclusion, empty default, no truncation.
- **Done when:** provider receives only selected existing plus inline evidence.

### T111 [ ] Implement evidence, annotation, label, and termination transactions
- **Depends:** T058, T105–T106
- **Files:** `integrations/src/persistence/run_mutations.rs`, tests.
- **Deliver:** operation-specific atomic writes with journal, lifecycle rules, stable IDs, version behavior.
- **Verify:** fault at each boundary; terminal evidence/annotation; terminal label/termination rejection.
- **Done when:** label/note/evidence do not change workflow CAS version.

### T112 [ ] Implement atomic event-attempt transaction
- **Depends:** T059, T105–T106, T110
- **Files:** `integrations/src/persistence/event_attempt.rs`, tests.
- **Deliver:** completed/rejected/error entry, evidence records/associations, provider/gate facts, optional mutation in one transaction; validate component and 2.5 MiB aggregate journal bounds before write.
- **Verify:** all failure-injection points, aggregate-overflow rejection with no partial write, and accurate evidence-recorded result.
- **Done when:** partial attempt/state/evidence is impossible.

### T113 [ ] Implement stale-evaluation CAS branch
- **Depends:** T112
- **Files:** `integrations/src/persistence/event_attempt.rs`, `integrations/tests/event_attempt_concurrency.rs`.
- **Deliver:** recheck workflow-state/lifecycle version after provider; apply transition only on match; append stale error attempt without transition when possible.
- **Verify:** two explicit-barrier requests; transition/termination invalidates; label/note/evidence does not.
- **Done when:** no write lock is held across provider invocation.

### T114 [ ] Implement guidance and per-run compatibility attempt persistence
- **Depends:** T058–T059, T081, T105–T106
- **Files:** `integrations/src/persistence/{guidance_attempt,compatibility_attempt}.rs` (inline tests).
- **Deliver:** completed/unsupported/lifecycle/error guidance facts plus completed/error compatibility findings, actual provider observations, and drift facts; no evidence/state/latch mutation.
- **Verify:** atomic append, persistence failure, missing provider, unchanged state/version, and drift observation; registration-wide check never uses this writer.
- **Done when:** every post-lookup guidance/compatibility request is explained when persistence remains available.

### T115 [ ] Implement ordered history query
- **Depends:** T046, T057, T105–T106
- **Files:** `integrations/src/persistence/history.rs`, tests.
- **Deliver:** immutable per-run sequence query, count/3 MiB byte pagination, correction links, and one-record progress guarantee.
- **Verify:** ordering across processes, no gaps after rollback, corruption detection, and maximum 2.5 MiB golden entry fits one history page/4 MiB CLI envelope without truncation.
- **Done when:** history never derives current state.

### T116 [ ] Implement consistent read-only audit export
- **Depends:** T015, T060, T105–T106, T109–T115
- **Files:** `integrations/src/export/{mod,state_json,journal_jsonl}.rs`, `schemas/export/v1/*.json`, `integrations/tests/export.rs`.
- **Deliver:** new/empty directory write from one consistent read snapshot, versioned state/journal files, no import.
- **Verify:** ordering, overwrite rejection, rollback/cleanup on partial filesystem failure, external locators not dereferenced.
- **Done when:** export is not competing authority.

### T117 [ ] Implement persistence corruption diagnostics
- **Depends:** T106–T116
- **Files:** `integrations/src/persistence/corruption.rs`, `integrations/tests/corruption.rs`, `integrations/tests/fixtures/corruption/*`.
- **Deliver:** rich errors for malformed DB/header/rows/snapshots/associations/sequences/schema versions.
- **Verify:** no silent repair/default/delete; fresh CLI/export logical authority, schema version, integrity, and row inventory unchanged after failed reads/writes; physical byte hash used only for immutable copied fixtures, never live WAL database.
- **Done when:** failure phase and trace correlation can be rendered.

### T118 [ ] Implement traced persistence boundary wrapper
- **Depends:** T099–T101, T104–T117
- **Files:** `integrations/src/persistence/traced.rs`, tests.
- **Deliver:** transaction intent, CAS/version check, commit/rollback/error events and bounded payloads.
- **Verify:** every production write path passes wrapper; read/migration categories follow trace contract.
- **Done when:** no direct production transaction bypass exists.

### T119 [ ] Add SQLite overlap and locking integration tests
- **Depends:** T107–T118
- **Files:** `integrations/tests/sqlite_overlap.rs`.
- **Deliver:** independent-run writes, handle races, run-create versus update/disable/restore in both writer orders, busy behavior, migration races, stale CAS, and post-kill reopen tests with explicit barriers.
- **Verify:** targeted test repeated under selected CI platforms; no query-then-mutate TOCTOU survives.
- **Done when:** tests use no timing-only sleep ordering.

## Phase 6 — CLI delivery and composition root

### T120 [ ] Implement trace-first CLI startup
- **Depends:** T006–T010, T096–T103
- **Files:** `cli/src/{main,startup}.rs`, `cli/tests/startup.rs`.
- **Deliver:** request ID and secure trace before help/version/parse/dispatch according to D010; rich init failure stderr.
- **Verify:** help, version, parse error, config error, and normal command each create expected trace; init failure does no DB/provider work.
- **Done when:** Clap cannot exit before required trace initialization.

### T121 [ ] Implement shared CLI argument primitives
- **Depends:** T004, T006–T008, T015–T016, T120
- **Files:** `cli/src/args.rs`, `cli/tests/args.rs`.
- **Deliver:** global flags, trace-first help/version/parse shell, and private operation-specific parser modules for planned flags; do not register application subcommands in production root yet.
- **Verify:** root help exposes zero application operations; private parser tests cover stable IDs/config, inputs/evidence, paged cursors/limits, `--active-runs-for`, disable `--warning-cursor`, and token-valued `--allow-active-runs`.
- **Done when:** exposure tasks can register one reviewed route without redefining shared grammar.

### T122 [ ] Implement CLI-to-core request DTO mappings
- **Depends:** T063–T083, T121
- **Files:** `cli/src/commands/{mod,provider,run,evidence,export}.rs`, `cli/tests/command_mapping.rs`.
- **Deliver:** delivery DTO validation and plain core operation requests without policy decisions.
- **Verify:** invalid syntax/pre-dispatch versus domain rejection boundary.
- **Done when:** CLI never selects transition or reinterprets verdict.

### T123 [ ] Implement sole composition root
- **Depends:** T096–T119, T120–T122
- **Files:** `cli/src/composition.rs`, `xtask/tests/architecture.rs`.
- **Deliver:** construct configuration, SQLite, provider process, clock/ID/digest, trace wrappers, and operations only here.
- **Verify:** architecture bypass canaries and `cargo check`.
- **Done when:** no other module instantiates concrete integration.

### T124 [ ] Implement traced operation dispatcher
- **Depends:** T054, T062–T083, T099–T103, T123
- **Files:** `cli/src/dispatch.rs`, `trace_payload.rs`, tests.
- **Deliver:** exactly one operation per intent, request/outcome trace envelopes, operation/request correlation, targeted decision-event sink.
- **Verify:** completed/rejected/error dispatch fixtures and no alternate route.
- **Done when:** all operations use same choke point.

### T125 [ ] Implement structured outcome schema and renderer
- **Depends:** T006, T047, T124
- **Files:** `cli/src/render/{mod,json,dto}.rs`, `schemas/cli/v1/outcome.schema.json`, tests.
- **Deliver:** versioned one-object envelope for every dispatched result.
- **Verify:** schema validation for all operation/outcome shapes and no trace/provider stream contamination.
- **Done when:** operation/request/trace/outcome always present.

### T126 [ ] Implement human renderer
- **Depends:** T047, T124
- **Files:** `cli/src/render/human.rs`, tests.
- **Deliver:** concise operation/run/gate/compatibility/evidence/trace presentation from same outcome.
- **Verify:** semantic parity table against structured renderer.
- **Done when:** renderer adds no policy.

### T127 [ ] Implement rich diagnostics and source-chain rendering
- **Depends:** T006, T047, T117, T120, T124
- **Files:** `cli/src/diagnostics.rs`, tests.
- **Deliver:** actionable request ID, phase, trace path, reason, provider/persistence context, and trace-init stderr.
- **Verify:** representative nested provider/config/persistence/trace failures.
- **Done when:** payloads need not be repeated because trace location is clear.

### T128 [ ] Implement stable exit and stdout/stderr behavior
- **Depends:** T006, T120, T125–T127
- **Files:** `cli/src/exit.rs`, `cli/tests/process_contract.rs`.
- **Deliver:** selected exit codes, one stdout object after dispatch, pre-dispatch stderr policy.
- **Verify:** byte-level stdout/stderr tests for every outcome and parse/init failures.
- **Done when:** provider streams never bypass engine.

### T129 [ ] Wire provider command handlers
- **Depends:** T063–T069, T122–T128
- **Files:** `cli/src/commands/provider.rs`.
- **Deliver:** private add/list/check/update/rename/disable/restore command adapters and renderable data; do not register them in production root.
- **Verify:** adapter tests only; provider add/list exposure waits T146–T147, provider check waits atomic T148–T152, lifecycle routes wait T155–T158.
- **Done when:** each adapter calls one core operation and root help still exposes none.

### T130 [ ] Wire run creation and read handlers
- **Depends:** T070–T074, T122–T128
- **Files:** `cli/src/commands/run.rs`.
- **Deliver:** private create/list/show/graph/history command adapters; do not register them in production root.
- **Verify:** adapter mapping tests; C5A grouped exposure waits T149–T152; show/graph expose T153–T154.
- **Done when:** safe reads have no provider path and root help still exposes none.

### T131 [ ] Wire evidence, annotation, label, and termination handlers
- **Depends:** T075–T078, T082, T122–T128
- **Files:** `cli/src/commands/{run,evidence}.rs`.
- **Deliver:** private evidence/annotation/label/termination command adapters; do not register them in production root.
- **Verify:** adapter mapping tests; C5A termination exposure waits T152 and audit-metadata exposures wait T159–T162.
- **Done when:** terminal allowance/denial stays core-owned and root help still exposes none.

### T132 [ ] Wire request, guidance, compatibility, and export handlers
- **Depends:** T079–T081, T083, T122–T128
- **Files:** `cli/src/commands/{run,export}.rs`.
- **Deliver:** private request/guidance/compatibility/export command adapters; do not register them in production root.
- **Verify:** adapter mapping tests; exposure waits T163–T166.
- **Done when:** no retry/revision token/gate bypass flag exists and root help still exposes none.

### T133 [ ] Implement production driver operation catalog
- **Depends:** T004, T062, T121–T132
- **Files:** `cli/src/driver_catalog.rs`, tests.
- **Deliver:** independently enumerable exposed operation IDs, initially empty, updated only by exposure tasks; extend T062 closure tool with driver and reachable-route sets.
- **Verify:** `cargo run -p xtask -- operation-coverage --mode baseline`; empty core=driver=reachable-route closure, uniqueness, and missing-route canary.
- **Done when:** driver metadata itself is not an application operation and no private adapter is reachable.

### T134 [ ] Prepare CLI/provider/trace/export schemas and author docs
- **Depends:** T084–T133
- **Files:** `schemas/index.json`, `examples/providers/README.md`, `../../cli-contract.md`, `../../operational-trace.md`, `../../export-contract.md`.
- **Deliver:** versioned generated/validated provider/trace/export contracts plus planned CLI schema; operation command docs remain marked planned until matching exposure task.
- **Verify:** schema generation is reproducible and working provider examples validate.
- **Done when:** no reader needs Rust source for stable contracts and docs do not claim unexposed commands.

## Phase 7 — Real provider fixtures and black-box harness

### T135 [ ] Create generic scenario-provider executable
- **Depends:** T013, T084–T095, T017
- **Files:** `test-support/providers/scenario-provider/{Cargo.toml,Cargo.lock,src/main.rs}`.
- **Deliver:** root-excluded standalone package using exact T001 fixture dependencies, reading protocol v1 and selecting explicit scenario config.
- **Verify:** `cargo test --manifest-path test-support/providers/scenario-provider/Cargo.toml --locked`; protocol schema validation; no product dependency.
- **Done when:** fixture imports no product crate and touches no engine DB.

### T136 [ ] Add generic graph/input scenario modes
- **Depends:** T135
- **Files:** `test-support/providers/scenario-provider/src/{graph,inputs}.rs`, `test-support/providers/scenario-provider/fixtures/{graphs,inputs}/*.json`.
- **Deliver:** linear/cycle/self-loop/zero-final/multi-final/initial-final/sink/ambiguous/invalid/guidance/input variants.
- **Verify:** provider-owned fixture tests and golden protocol results.
- **Done when:** every graph/input facet can be selected without changing test code.

### T137 [ ] Add gate/evidence/guidance/compatibility scenario modes
- **Depends:** T135–T136
- **Files:** `test-support/providers/scenario-provider/src/{gates,evidence,guidance,compatibility}.rs`, `test-support/providers/scenario-provider/fixtures/roles/*.json`.
- **Deliver:** pass/fail/mixed/exact-set violations/evidence variants/evaluation error/incompatibility/mixed support/live guidance.
- **Verify:** standalone role matrix tests.
- **Done when:** all provider semantic branches are controllable deterministically.

### T138 [ ] Add provider process-failure modes
- **Depends:** T135
- **Files:** `test-support/providers/scenario-provider/src/process_failures.rs`, `test-support/providers/scenario-provider/fixtures/process/*.bin`.
- **Deliver:** malformed JSON, extra/missing output, wrong major, nonzero, signal, timeout, oversized streams, invalid UTF-8 per selected contract.
- **Verify:** direct process assertions and cleanup.
- **Done when:** no mode uses mocks.

### T139 [ ] Add explicit provider barrier and invocation ledger
- **Depends:** T135–T138
- **Files:** `test-support/providers/scenario-provider/src/{barrier,ledger}.rs`.
- **Deliver:** filesystem/pipe synchronization, recorded request/role/CWD/argv/digest-mode facts for deterministic overlap and no-invocation proof.
- **Verify:** barrier start/release/kill and concurrent ledger integrity tests.
- **Done when:** no concurrency test requires timing-only sleeps.

### T140 [ ] Create reference software-change provider graph
- **Depends:** T013, T084–T095
- **Files:** `test-support/providers/reference-provider/{Cargo.toml,Cargo.lock,src/main.rs,src/graph.rs}`.
- **Deliver:** root-excluded standalone package with exact T001 fixture dependencies and required states/events/transitions/gates/inputs/guidance.
- **Verify:** `cargo test --manifest-path test-support/providers/reference-provider/Cargo.toml --locked`; graph test against `../../reference-workflow.md`; no product dependency.
- **Done when:** software concepts exist only in fixture/provider.

### T141 [ ] Implement reference provider gate and evidence policy
- **Depends:** T140
- **Files:** `test-support/providers/reference-provider/src/{gates,evidence}.rs`, `test-support/providers/reference-provider/fixtures/*.json`.
- **Deliver:** missing/invalid artifact checks, revision linkage, verdict consistency, implementation/validation evidence, provider evidence.
- **Verify:** provider-owned semantic tests for pass/fail/revision/malformed cases.
- **Done when:** engine sees only generic protocol data.

### T142 [ ] Implement reference guidance, drift, and compatibility modes
- **Depends:** T140–T141
- **Files:** `test-support/providers/reference-provider/src/{guidance,compatibility}.rs`, `test-support/providers/reference-provider/fixtures/{config,versions}/*.json`.
- **Deliver:** advisory evidence recommendations, build/digest drift, stored-gate incompatibility, changed graph for new runs.
- **Verify:** provider-owned role tests.
- **Done when:** active graph is never mutated by fixture.

### T143 [ ] Build isolated E2E sandbox
- **Depends:** T007, T096–T101, T104–T119, T120–T134
- **Files:** `cli/tests/e2e.rs`, `cli/tests/support/{mod,sandbox}.rs`.
- **Deliver:** registered E2E integration-test root plus private home/config/DB/trace/provider CWD, no caller config/network/shared fixture, preserved failure artifacts.
- **Verify:** harness self-test with two independent sandboxes.
- **Done when:** every command runs as fresh production CLI process.

### T144 [ ] Build CLI process runner and structured parser
- **Depends:** T006, T125, T128, T143
- **Files:** `cli/tests/support/{cli,outcome}.rs`.
- **Deliver:** invoke built binary, capture bytes/exit, parse one envelope, run human mode, record transcript.
- **Verify:** self-tests for completed/rejected/error/pre-dispatch and malformed extra stdout.
- **Done when:** no in-process command handler call exists.

### T145 [ ] Build trace parser, runtime coverage recorder, fixture helpers, and close C2
- **Depends:** T061, T099–T101, T133, T135–T144
- **Files:** `cli/tests/support/{trace,coverage,provider,sqlite}.rs`.
- **Deliver:** semantic trace assertions, runtime E2E/trace set collectors extending T062/T133 closure tool, provider registration helpers, migration/corruption helpers, narrowly schema-valid tombstoned-registration prerequisite setup for pre-T157 list/check tests, and Unix `RLIMIT_FSIZE` wrapper. Setup never counts as disable/restore coverage and is repeated via CLI in T175.
- **Verify:** full substrate smoke; `operation-coverage --mode baseline`; zero application routes; stale-artifact/request-ID checks; tombstone setup validates production schema and cannot register operation evidence; `EFBIG` wrapper self-tests.
- **Done when:** C2 candidate has production substrate/harness, no public operation, no manual coverage labels, and stops for orchestrator commit `test(e2e): establish black-box harness`.

## Phase 8 — Expose each application operation through complete vertical slice

Each exposure task first registers production route and both runtime catalog IDs in a deliberately uncommitted candidate tree, then runs production CLI E2Es/closure and updates docs/coverage. After facets pass, task stops for README-authorized orchestrator commit; delegation is forbidden. Failed candidate remains unpublished and must remove/fix registration before handoff. Tasks T168–T183 add aggregate, interaction, stress, and independent-model proof; no exposure task may defer an operation's mandatory facet to them. Same-owner atomic groups follow README protocol: one owner executes full range while orchestrator maintains `[~]`; final owner report proves group-ready and orchestrator alone marks `[x]`. Every exposure owns `quality/facets/v1/<operation-id>.json` validated by T004 schema; manifest starts with all applicable rows open and records exact E2E/trace evidence before commit. Every MVP application operation touches persistence, so every manifest must close `Trace persistence boundary` using that exposure's production CLI trace (attempted read/transaction, applicable version check, and read/commit/rollback outcome). Provider users also close `Trace provider boundary`. T182 only repeats already-closed rows and cannot supply first proof.

### T146 [ ] Prepare `provider.add` production route and required facets
- **Depends:** T063, T107, T120–T129, T143–T145
- **Files:** provider command/catalogs; `cli/tests/e2e/provider_add.rs`; `quality/facets/v1/provider.add.json`; docs/coverage.
- **Deliver:** checkpoint-working-tree production route/catalog ID plus explicit config registration, stable ID, duplicate-handle rejection, human/JSON/trace; this intermediate tree must not be committed.
- **Verify:** `cargo test -p loop-engine-cli --test e2e provider_add`; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open provider.add`; exposed closure waits T147.
- **Done when:** add response/trace/rejection facets pass; fresh catalog persistence proof remains explicitly open until T147; T146 stays `[~]`.

### T147 [ ] Expose `provider.add` and `provider.list` checkpoint
- **Depends:** T064, T107, T129, T143–T146
- **Files:** provider command/catalogs; `provider_list.rs`; `quality/facets/v1/{provider.add,provider.list}.json`; docs/coverage.
- **Deliver:** finalize grouped add/list routes and catalog IDs; enabled/tombstoned registration list, zero-row `--active-runs-for`, invalid filter, fresh add persistence, and D008 count ceiling/page-byte stop/cursor/progress/no-truncation behavior; tombstone setup is not disable coverage.
- **Verify:** production CLI E2Es for add/list and every pagination row; successful/rejected add verified by fresh list; no run journal; closure.
- **Done when:** C3 closes, no provider ledger invocation occurs for list, and same owner reports T146–T147 ready; orchestrator marks both `[x]`.

### T148 [ ] Prepare `provider.check` route and non-run facets
- **Depends:** T065, T084–T095, T102, T129, T135–T145, T146–T147
- **Files:** provider command/catalogs; `provider_check.rs`; `quality/facets/v1/provider.check.json`; docs/coverage.
- **Deliver:** checkpoint route/catalog ID; machine-readable facet inventory; valid completion; role-valid invalid finding; missing/tombstoned registration; paged `--active-runs` count/byte/cursor semantics; missing executable; timeout; crash/nonzero/signal; malformed/wrong-major/invalid-UTF-8 protocol; oversized output/streams; conformance diagnostics; invocation/trace facts; intermediate tree uncommitted.
- **Substeps:** add failing inventory-backed E2Es; close valid/finding cases; close process/protocol/bound cases; close render/trace/docs; leave `--active-runs` rows open for T152.
- **Verify:** targeted E2E covers every non-run provider-check facet; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open provider.check`; no generic rejection.
- **Done when:** non-run rows pass, active-run rows remain explicitly open, and T148 stays `[~]` until T152.

### T149 [ ] Prepare `run.create` production route and required facets
- **Depends:** T070, T086, T091–T093, T108, T130, T135–T145, T148
- **Files:** run command/catalogs; `run_create.rs`; `quality/facets/v1/run.create.json`; docs/coverage.
- **Deliver:** checkpoint-working-tree route/catalog ID plus machine-readable facet inventory; zero/valid/invalid inputs; provider completion, role-valid input denial, and `validate_inputs` evaluation error with no run/journal; missing registration/executable; timeout; crash/nonzero/signal; malformed/wrong-major/invalid-UTF-8/oversized output; description/validation drift; canonical field matrix; invalid graph; initial-final; rejected/error creation writes no run/journal; trace; intermediate tree uncommitted.
- **Substeps:** add failing input/provider/graph/atomicity rows; close provider-free rejection semantics; close provider failures; close graph/drift; close render/trace/docs; leave fresh list/history rows open for T152.
- **Verify:** targeted E2E; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open provider.check,run.create`; final fresh-process state/journal and exposed closure wait T152.
- **Done when:** all creation-owned rows pass, cross-command rows remain explicitly open, and T149 stays `[~]` until T152.

### T150 [ ] Prepare `run.list` and fresh creation-state facets
- **Depends:** T071, T109, T130, T143–T149
- **Files:** run command/catalogs; `run_list.rs`; `quality/facets/v1/run.list.json`; docs/coverage.
- **Deliver:** checkpoint-working-tree route/catalog ID; active default, terminal/all filters, another CWD, labels, invalid filters, D008 default/max limit and empty/malformed/wrong-version/filter-mismatched cursor; fresh-process list verifies creation state; intermediate tree remains uncommitted.
- **Verify:** production CLI create/list E2Es; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open provider.check,run.create,run.list`; final run-journal/lifecycle exposed closure waits T152.
- **Done when:** stored snapshot/inputs/registration survive restart and T150 stays `[~]` until T152.

### T151 [ ] Prepare `run.terminate` and terminal lifecycle facets
- **Depends:** T082, T111, T131, T143–T150
- **Files:** run command/catalogs; `run_terminate.rs`; `quality/facets/v1/run.terminate.json`; docs/coverage.
- **Deliver:** checkpoint-working-tree route/catalog ID; active termination/note, repeat and final-run rejection, terminal listing, no reopen surface, exact journal/trace; intermediate tree remains uncommitted.
- **Verify:** targeted termination/list E2E; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open provider.check,run.create,run.list,run.terminate`; final history proof waits T152 and show proof remains T153.
- **Done when:** terminate-owned lifecycle facets have production fixtures and T151 stays `[~]` until T152.

### T152 [ ] Expose atomic C4/C5A check/create/list/terminate/history checkpoint
- **Depends:** T074, T115, T130, T143–T151
- **Files:** provider/run command catalogs; `provider_check.rs`; `run_create.rs`; `run_list.rs`; `run_terminate.rs`; `run_history.rs`; `quality/facets/v1/{provider.check,run.create,run.list,run.terminate,run.history}.json`; docs/coverage.
- **Deliver:** finalize grouped provider.check/create/list/terminate/history routes and catalog IDs; zero/mixed registration-wide findings plus first-row/later-row `check_compatibility` evaluation errors that fail whole page without partial report/journal; ordered history with D008 default/max limit and empty/malformed/wrong-version/filter-mismatched cursor; fresh creation/termination state/journal; active/final/terminated listing; invalid/not-found queries; provider-free restart.
- **Substeps:** close list/history cross-command rows; close terminate/list lifecycle rows; close active-run compatibility rows; prove no journal fan-out/latch; run four-set closure; update docs; report T148–T152 ready for orchestrator to mark `[x]` together.
- **Verify:** provider-check plus create/list/terminate/history CLI E2Es; history and active-compatibility cover count ceiling, byte stop, cursor progress, maximum journal row, multi-page mixed snapshots, and no record truncation; ledger proves one registration resolution, exactly one describe, at most nine compatibility calls, and at most ten trace reservations per page; universal closure; no journal fan-out/replay/hidden execution.
- **Done when:** C4/C5A candidate is ready; owner stops for orchestrator commit `feat(run): expose provider check and run foundation`.

### T153 [ ] Expose `run.show` lifecycle read slice
- **Depends:** T072, T109, T130, T143–T152
- **Files:** run command/catalogs; `run_show.rs`; `quality/facets/v1/run.show.json`; docs/coverage.
- **Deliver:** candidate route/catalog ID; active, neutral final, initial-final, zero-final ongoing, non-final sink, terminated, static guidance/no-guidance, gates, live capability, empty events, missing provider/not-found.
- **Verify:** targeted production-CLI E2E, provider ledger empty, four-set closure before authorized exposure commit.
- **Done when:** show satisfies complete lifecycle/read matrix without provider execution.

### T154 [ ] Expose `run.graph` and close C5
- **Depends:** T073, T109, T130, T143–T153
- **Files:** run command/catalogs; `run_graph.rs`; `quality/facets/v1/run.graph.json`; docs/coverage.
- **Deliver:** candidate route/catalog ID; complete stored projection/revision for active/final/terminated runs after provider graph drift and missing provider; invalid/not-found run rejection.
- **Verify:** targeted production-CLI E2E; graph bytes/meaning stable; four-set closure before authorized exposure commit.
- **Done when:** active run never consults latest description and C5 closes.

### T155 [ ] Expose `provider.update`
- **Depends:** T066, T107, T129, T143–T154
- **Files:** provider command/catalogs; `provider_update.rs`; `quality/facets/v1/provider.update.json`; docs/coverage.
- **Deliver:** same-ID config replacement, affected count plus paged-impact link without approval flag, config-revision increment, CWD/argv/timeout update, fresh-process verification.
- **Verify:** valid/rejected targeted E2E and trace; fresh provider list proves changed/unchanged catalog with no run journal; existing runs retain registration ID/stored graph; closure.
- **Done when:** catalog update and stable binding are proved; active provider invocation through new config remains assigned to T163/T175.

### T156 [ ] Expose `provider.rename`
- **Depends:** T067, T107, T129, T143–T155
- **Files:** provider command/catalogs; `provider_rename.rs`; `quality/facets/v1/provider.rename.json`; docs/coverage.
- **Deliver:** unique-handle rename and stable existing-run binding.
- **Verify:** valid/rejected targeted E2E; fresh provider list proves changed/unchanged catalog with no run journal; closure.
- **Done when:** old handle released without ID change.

### T157 [ ] Expose `provider.disable`
- **Depends:** T068, T107, T129, T143–T156
- **Files:** provider command/catalogs; `provider_disable.rs`; `quality/facets/v1/provider.disable.json`; docs/coverage.
- **Deliver:** first/intermediate/final bounded warning pages, final-page-only opaque ack token, early-cursor/digest rejection, changed-set rejection, tombstone, released handle, safe reads/termination.
- **Verify:** CLI must page all named IDs before token appears; first/intermediate/forged/stale token cannot mutate; final token succeeds; fresh list proves tombstone/unchanged catalog with no run journal; closure.
- **Done when:** no referenced registration row is deleted.

### T158 [ ] Expose `provider.restore`
- **Depends:** T069, T107, T129, T143–T157
- **Files:** provider command/catalogs; `provider_restore.rs`; `quality/facets/v1/provider.restore.json`; docs/coverage.
- **Deliver:** restore exact ID with free handle/config, occupied-handle rejection, former-handle reuse isolation.
- **Verify:** valid/rejected targeted E2E; fresh provider list proves restored/unchanged catalog with no run journal; stable run binding remains; closure.
- **Done when:** C6 catalog lifecycle is fully proved; resumed provider-dependent use remains assigned to T163/T175.

### T159 [ ] Prepare `run.evidence.add` production route and required facets
- **Depends:** T075, T110–T111, T131, T143–T158
- **Files:** evidence command/catalogs; `evidence_add.rs`; `quality/facets/v1/run.evidence.add.json`; docs/coverage.
- **Deliver:** checkpoint-working-tree production route/catalog ID plus active/final/terminated append, stable ID, same-locator revision, exact D008 empty/control/NUL/oversize locator rejection without URI/path/CWD interpretation, journal atomicity; this intermediate tree must not be committed.
- **Verify:** targeted E2E and fault injection; `cargo run -p xtask -- operation-coverage --mode candidate --allow-open run.evidence.add`; final fresh-process list and exposed closure wait T160.
- **Done when:** append response/atomicity/lifecycle facets pass; fresh inventory proof remains explicitly open until T160; T159 stays `[~]`.

### T160 [ ] Expose `run.evidence.add` and `run.evidence.list` checkpoint
- **Depends:** T076, T110, T131, T143–T159
- **Files:** evidence command/catalogs; `evidence_list.rs`; `quality/facets/v1/{run.evidence.add,run.evidence.list}.json`; docs/coverage.
- **Deliver:** finalize grouped evidence add/list routes and catalog IDs; provider-free inventory/associations across active/final/terminated lifecycle and restart; invalid/not-found; D008 default/max limit and empty/malformed/wrong-version/filter-mismatched cursor; fresh list verifies append persistence.
- **Verify:** production CLI add/list E2Es including every evidence pagination row; missing provider ledger empty; closure.
- **Done when:** C7 evidence checkpoint begins with stable IDs available for selection and same owner reports T159–T160 ready; orchestrator marks both `[x]`.

### T161 [ ] Expose `run.annotate`
- **Depends:** T077, T111, T131, T143–T160
- **Files:** run command/catalogs; `run_annotate.rs`; `quality/facets/v1/run.annotate.json`; docs/coverage.
- **Deliver:** active/final/terminated note, actor metadata, correction link, no authority/state/version change.
- **Verify:** targeted E2E and actor-neutral comparison; closure.
- **Done when:** append-only correction is visible in history for every lifecycle.

### T162 [ ] Expose `run.label`
- **Depends:** T078, T111, T131, T143–T161
- **Files:** run command/catalogs; `run_label.rs`; `quality/facets/v1/run.label.json`; docs/coverage.
- **Deliver:** active change/remove, duplicate labels, final/terminated rejection, journal, no run rebinding.
- **Verify:** targeted E2E and closure.
- **Done when:** C7 audit metadata family is fully closed.

### T163 [ ] Expose `run.request`
- **Depends:** T079, T093, T102, T110–T113, T118–T119, T132, T135–T145, T149–T162
- **Files:** run command/catalogs; `run_request.rs`; `cli/tests/support/{provider,sqlite,trace}.rs`; `quality/facets/v1/run.request.json`; docs/coverage.
- **Deliver:** machine-readable facet inventory; gate-free/gated; all verdict variants; inline/selected/provider evidence; unknown/final/terminated; self-loop/cycle/final; selected unsupported plus supported continuity; missing/tombstoned provider where gate-free event completes and gated event errors; current updated/restored config invocation; provider role-valid denial, timeout/crash/nonzero/signal/malformed/wrong-major/invalid-UTF-8/oversized output; stale CAS; exact journal/trace.
- **Substeps:** register route/catalog IDs in uncommitted candidate; close selection/pre-provider rows; gate-free/lifecycle rows; verdict/evidence rows; provider failure/compatibility rows; stale/atomicity rows; render/trace/docs/catalog closure.
- **Verify:** inventory-driven request provider/gate/compatibility/lifecycle suite, fault injection, deterministic overlap, fresh history, and four-set closure before exposure commit.
- **Done when:** C8 candidate is fully closed; owner stops for orchestrator commit `feat(run): expose run.request`.
- **Stop:** any applicable facet remains untested; keep candidate uncommitted and operation unpublished.

### T164 [ ] Expose `run.guidance`
- **Depends:** T080, T094, T102, T114, T132, T142–T145, T163
- **Files:** run command/catalogs; `run_guidance.rs`; `quality/facets/v1/run.guidance.json`; docs/coverage.
- **Deliver:** machine-readable facet inventory; completed advisory; stored capability unsupported; provider-declared stored-guidance incompatibility; supported continuity; final/terminated rejection; evaluation error; missing/tombstoned registration/executable; timeout/crash/nonzero/signal/malformed/wrong-major/invalid-UTF-8/oversized output; evidence recommendation; no evidence/state mutation; journal/trace.
- **Substeps:** close advisory/unsupported/incompatibility rows; lifecycle rows; provider process/protocol rows; fresh-history mutation proof; render/trace/docs/catalog closure.
- **Verify:** complete inventory-driven guidance provider/compatibility/lifecycle E2E, fresh `run.history` for every post-lookup outcome, and closure before exposure commit.
- **Done when:** passive show remains provider-free and incompatibility is domain rejection.

### T165 [ ] Expose `run.compatibility`
- **Depends:** T081, T095, T102, T114, T132, T142–T145, T163–T164
- **Files:** run command/catalogs; `run_compatibility.rs`; `quality/facets/v1/run.compatibility.json`; docs/coverage.
- **Deliver:** machine-readable facet inventory; mixed non-latching findings; invalid/not-found run and final/terminated rejection; current registration/digest; role-valid incompatible finding and evaluation error with no latch/state mutation; atomic compatibility-attempt journal records actual provider facts/drift for every post-lookup outcome; missing/tombstoned registration/executable; timeout/crash/nonzero/signal/malformed/wrong-major/invalid-UTF-8/oversized output; selected unsupported rejection; supported request/guidance and gate-free continuity; repeated non-latching check.
- **Substeps:** close report/finding rows; lifecycle rows; provider process/protocol rows; atomic history/drift rows; cross-operation continuity; no-latch/state proof; render/trace/docs/catalog closure.
- **Verify:** complete compatibility E2E, fresh history for completed/error/drift observations, unchanged state/version, no registration-wide journal fan-out, and closure.
- **Done when:** C9 advisory compatibility is closed.

### T166 [ ] Expose `run.export`
- **Depends:** T083, T116, T132, T143–T165
- **Files:** export command/catalogs; `run_export.rs`; `quality/facets/v1/run.export.json`; `schemas/export/v1/*.json`; `../../export-contract.md`; docs/coverage.
- **Deliver:** versioned active/final/terminated state/journal export to new/empty directory, invalid/not-found run rejection, overwrite/partial-filesystem error behavior, D006-compatible schema policy, no import/dereference.
- **Verify:** targeted valid/invalid/not-found/overwrite/partial-failure E2E, schema validation, and closure.
- **Done when:** C10 export closes.

## Phase 9 — Cross-operation black-box acceptance

### T167 [ ] Enforce universal operation/driver/E2E/trace closure
- **Depends:** T145–T166
- **Files:** `cli/tests/e2e/coverage_closure.rs`, `xtask/src/operation_coverage.rs`, `quality/coverage/v1/{schema.json,final-catalog.json}`.
- **Deliver:** finalize T062/T133/T145 closure tool by independently collecting core/driver/E2E/trace sets, validating one closed facet manifest per exposed ID, comparing exact equality, and asserting final set equals D004's 21 IDs.
- **Verify:** `cargo run -p xtask -- operation-coverage --mode final`; deliberate missing driver/E2E/trace/open-facet/stale-artifact canaries fail.
- **Done when:** all 21 IDs have invocation references, not labels.

### T168 [ ] Cover full graph semantic family
- **Depends:** T149–T163
- **Files:** `cli/tests/e2e/graph_semantics.rs`.
- **Deliver:** linear/cycles/self-loops, zero/one/multiple finals, initial-final, final outgoing rejection, non-final sink, unknown/ambiguous defense.
- **Verify:** production CLI, real provider, persistence, fresh reads/history/traces.
- **Done when:** graph facet map has no gap.

### T169 [ ] Prove human/structured semantic parity and outcome taxonomy
- **Depends:** T146–T166
- **Files:** `cli/tests/e2e/outcomes.rs`.
- **Deliver:** every top-level outcome/reason family, one-object stdout, exits, same run semantics, provider stream isolation.
- **Verify:** process-level byte assertions.
- **Done when:** no fourth outcome or privileged mode exists.

### T170 [ ] Cover complete provider execution failure family
- **Depends:** T148–T149, T163–T165
- **Files:** `cli/tests/e2e/provider_failures.rs`.
- **Deliver:** aggregate repetition across check/create/gated request/guidance/compatibility: role-valid evaluation errors, missing/tombstoned executable, wrong major, timeout, nonzero, signal, malformed/missing/oversize/invalid-UTF-8 outputs, bounded streams.
- **Verify:** no case is first introduced here; creation error has no run/journal, paged check has no partial report/fan-out, per-run check has no latch/state/journal, attempt journaling is exact, rich error/trace.
- **Done when:** gate-free/provider-free operations remain usable and exposure-task facet inventory matches aggregate suite.

### T171 [ ] Cover creation inputs and canonical graph drift
- **Depends:** T148–T149, T154
- **Files:** `cli/tests/e2e/creation_and_digest.rs`.
- **Deliver:** zero/required/optional/invalid/undeclared inputs, no topology from values, description-validation executable drift, canonical field matrix.
- **Verify:** no run/journal on rejected/error creation.
- **Done when:** alternate topology requires separate registration.

### T172 [ ] Cover evidence selection, associations, and correction
- **Depends:** T159–T163
- **Files:** `cli/tests/e2e/evidence_flow.rs`.
- **Deliver:** empty default, missing ID, oversized context pre-provider, only selected records, inline/provider pass/fail evidence, same-locator revisions, remap convention.
- **Verify:** fresh inventory/history and provider request ledger.
- **Done when:** no auto-selection/truncation/dereference occurs.

### T173 [ ] Cover journal/state/evidence atomicity with fault injection
- **Depends:** T108, T111–T118, T149, T151, T159, T161–T165
- **Files:** `cli/tests/e2e/atomicity.rs`, `test-support/sqlite/faults/*.sql`.
- **Deliver:** abort each creation/mutation/attempt write boundary—including post-lookup `run.guidance` and `run.compatibility` attempt journal commits under I14—and inspect via fresh CLI.
- **Verify:** old-or-new complete state only; response never falsely claims recording; deterministic fault-injection rollback/atomicity proof for every post-lookup `run.guidance` and `run.compatibility` attempt journal boundary.
- **Done when:** every durable mutation/attempt class, including guidance and per-run compatibility attempts, has rollback evidence under I14.

### T174 [ ] Cover lifecycle family end to end
- **Depends:** T149–T166
- **Files:** `cli/tests/e2e/lifecycle.rs`.
- **Deliver:** active/final/terminated, neutral final meaning, zero-final, sink, termination, repeated denial, terminal empty events, fixed label, annotation/evidence, guidance/compat rejection.
- **Verify:** fresh process after every state change.
- **Done when:** no pause/reopen/delete semantics surface.

### T175 [ ] Cover provider drift and stable registration identity
- **Depends:** T149–T158, T163–T165
- **Files:** `cli/tests/e2e/provider_drift.rs`.
- **Deliver:** handle rename/reuse, update executable/argv/CWD/version/digest/graph, nonempty paged impact reads, CLI-created disable/restore tombstones repeating setup cases, fixed stored projection, current config invocation, journal facts.
- **Verify:** another caller CWD and missing/tombstoned provider paths.
- **Done when:** locator/digest/handle never rebind identity.

### T176 [ ] Cover capability-scoped compatibility continuity
- **Depends:** T163–T165, T175
- **Files:** `cli/tests/e2e/compatibility.rs`.
- **Deliver:** repeat registration-wide zero/mixed findings with no fan-out; per-run mixed/error/drift findings with ordered attempt journal; unsupported gated request; guidance incompatibility; supported/gate-free continuity; missing-provider errors; non-latching repeat.
- **Verify:** no case first introduced here; per-run checks append observations without state/version/latch; safe reads/annotation/termination remain available.
- **Done when:** exposure-task inventory matches aggregate suite and no active graph migration/bypass exists.

### T177 [ ] Cover configuration precedence and caller-CWD independence
- **Depends:** T096–T098, T146–T166
- **Files:** `cli/tests/e2e/configuration.rs`.
- **Deliver:** global/project/CLI precedence, malformed/forbidden config, ancestor boundary, defaults-only distinction, stored registration resolution.
- **Verify:** isolated homes and multiple CWDs.
- **Done when:** project config cannot redefine existing run/provider.

### T178 [ ] Cover accidental overlap and stale evaluations
- **Depends:** T113, T119, T139, T163
- **Files:** `cli/tests/e2e/concurrency.rs`.
- **Deliver:** two gated requests, termination/state change during barrier, label/note/evidence during barrier, run creation racing update/disable/restore in both writer orders, independent-run writes, process kill.
- **Verify:** explicit barriers, timeline artifacts, fresh state/history, CAS trace.
- **Done when:** one conflicting transition max; metadata writes do not stale valid evaluation.

### T179 [ ] Freeze and test migration v1 fixture discipline
- **Depends:** T104–T119, T143–T145, T149–T154
- **Files:** `test-support/sqlite/v0001.*`, `cli/tests/e2e/migrations.rs`.
- **Deliver:** immutable released-schema fixture, empty/latest/future/concurrent/interrupted migration scenarios and fixture-generation provenance.
- **Verify:** CLI queries preserve all authority after migration.
- **Done when:** future migrations must add, never rewrite, fixture.

### T180 [ ] Cover persistence corruption behavior
- **Depends:** T117, T143–T145, T150–T154
- **Files:** `test-support/sqlite/corrupt/*`, `cli/tests/e2e/corruption.rs`.
- **Deliver:** invalid header/truncation/malformed snapshot/association/lifecycle/sequence/future schema cases.
- **Verify:** rich persistence-phase errors, no provider execution/mutation/silent repair.
- **Done when:** corruption claims are public-CLI observed.

### T181 [ ] Cover audit export consistency and failure safety
- **Depends:** T116, T166
- **Files:** `cli/tests/e2e/export.rs`.
- **Deliver:** consistent state/journal/evidence/provider observations, ordering, external locators, overwrite and partial-filesystem failure.
- **Verify:** exported schemas validate; DB unchanged; no import command.
- **Done when:** export never claims replay/mobility.

### T182 [ ] Cover operational trace contract and resilience
- **Depends:** T099–T102, T118, T120, T146–T181
- **Files:** `cli/tests/e2e/{trace_contract,trace_resilience}.rs`.
- **Deliver:** permissions, one file/request, all payloads, no environment, init failure/no effects, crash markers, base/call reservation exhaustion, provider-read late sink failure, durable-annotation late sink failure, count/byte rotation, concurrent invocations.
- **Verify:** semantic parsing; provider-check pages at ten calls with cursor; mutation case proves completed envelope plus fresh-process annotation/history after `EFBIG`; concurrent actual+reserved bytes never exceed cap.
- **Done when:** every outcome matches trace limits, committed mutation stays truthful, and open traces survive within hard bound.

### T183 [ ] Add independent model-based black-box testing
- **Depends:** T135–T145, T149–T182
- **Files:** `cli/tests/e2e/model_based.rs`, `cli/tests/support/reference_model.rs`.
- **Deliver:** independent model and generated graph/action sequences through fresh CLI processes.
- **Verify:** preserved seed/provider/project/transcript/exports/traces and replay command on failure.
- **Done when:** reference model imports no product transition code.

### T184 [ ] Audit excluded APIs and architecture behavior
- **Depends:** T017–T183
- **Files:** `xtask/src/architecture.rs`, `xtask/tests/architecture.rs`, `cli/tests/e2e/excluded_surface.rs`, `quality/evidence/v1/exclusions.json`.
- **Deliver:** proof of no DSL/agent/daemon/async/replay/sandbox/discovery/claims/retries/reopen/delete/import/migration/bypass/SDK requirement and no software policy in core.
- **Verify:** command help/schema/catalog/dependency/source-boundary checks.
- **Done when:** every explicit non-goal in README and foundation has evidence.

## Phase 10 — Required reference software-change acceptance

### T185 [ ] Prove reference behaviors 1–4
- **Depends:** T140–T142, T149–T184
- **Files:** `cli/tests/reference.rs`, `cli/tests/reference/creation_and_rejection.rs`.
- **Deliver:** registered reference integration-test root plus creation/safe inspection, forward happy path baseline, missing output rejection, invalid output rejection.
- **Verify:** targeted reference E2E `creation_and_rejection`; separate processes, stored graph, state/history/trace.
- **Done when:** coverage rows 1–4 cite invocation IDs.

### T186 [ ] Prove reference behaviors 5–9
- **Depends:** T185
- **Files:** `cli/tests/reference/revision_cycles.rs`.
- **Deliver:** design/plan/implementation/validation revision cycles and event/verdict consistency.
- **Verify:** targeted reference E2E `revision_cycles`; every cycle preserves prior evidence and returns through explicit graph edge.
- **Done when:** rows 5–9 cite runtime evidence.

### T187 [ ] Prove reference behaviors 10–13
- **Depends:** T185–T186
- **Files:** `cli/tests/reference/evidence_drift_compatibility.rs`.
- **Deliver:** append-only same-path evidence, restart/handoff, provider drift, explicit incompatibility and safe recovery operations.
- **Verify:** targeted reference E2E `evidence_drift_compatibility`; fresh processes, changed provider, fixed graph, history/trace.
- **Done when:** rows 10–13 cite runtime evidence.

### T188 [ ] Prove reference behaviors 14–17
- **Depends:** T185–T187
- **Files:** `cli/tests/reference/guidance_actor_interaction.rs`.
- **Deliver:** cold guidance/evidence selection, actor neutrality, journal/state consistency, cross-CWD interaction/terminal annotation.
- **Verify:** targeted reference E2E `guidance_actor_interaction`; identical decision under actor metadata change and provider-free reads.
- **Done when:** rows 14–17 cite runtime evidence.

### T189 [ ] Prove reference behaviors 18–21
- **Depends:** T185–T188
- **Files:** `cli/tests/reference/attempt_resolution_visibility.rs`.
- **Deliver:** attempt evidence categories, stable registration resolution, automation envelope/exits, operational trace and trace-init no-effect.
- **Verify:** targeted reference E2E `attempt_resolution_visibility`; completed/rejected/unknown/lifecycle/error attempts and exact trace correlation.
- **Done when:** rows 18–21 cite runtime evidence.

### T190 [ ] Generate exact reference 1–21 and C11 acceptance report
- **Depends:** T183, T185–T189
- **Files:** `xtask/src/reference_report.rs`, `xtask/tests/reference_report.rs`, `quality/evidence/v1/reference-report.schema.json`.
- **Deliver:** generator/schema plus 21 stable tracked evidence keys; after authorized candidate commit, invoke generator against immutable SHA to emit untracked/CI artifact with invocation/trace references; never embed self-SHA in tracked tree.
- **Verify:** pre-commit missing/duplicate/manual-only canaries fail; T183 model-based suite evidence is green and linked; post-commit report records passing `<sha>/reference-1-21` status.
- **Done when:** all 21 reference behaviors plus model-based acceptance close C11 in external immutable status.

## Phase 11 — Final quality, documentation, and publication hardening

### T191 [ ] Generate complete invariant and facet evidence report
- **Depends:** T167–T190
- **Files:** `xtask/src/acceptance_report.rs`, `xtask/tests/acceptance_report.rs`, `quality/evidence/v1/acceptance-report.schema.json`, `coverage.md`.
- **Deliver:** report generator/schema and stable tracked keys for I1–I47, operation facets, reference 1–21, exclusions, commands, toolchain/fixture digests; post-commit artifact supplies actual commit digest.
- **Verify:** pre-commit stale/missing/duplicate canaries; stop for authorized T191 commit; run immutable-SHA report/status command on committed generator.
- **Done when:** every stable key resolves in external artifact to passing runtime or justified deterministic evidence; failed report opens authorized corrective commit.

### T192 [ ] Finalize provider-author documentation and examples
- **Depends:** T134–T190
- **Files:** `../../provider-protocol-v1.md`, `schemas/provider/v1/*.json`, `examples/providers/**`, `../../configuration.md`, `../../operational-trace.md`.
- **Deliver:** registration, five roles, bounds, errors, conformance, graph/input/evidence/guidance/compatibility, no sandbox/state authority.
- **Verify:** examples execute and pass `provider.check` from clean checkout.
- **Done when:** author needs no internal source knowledge.

### T193 [ ] Finalize user/operator CLI documentation
- **Depends:** T146–T190
- **Files:** `../../../README.md`, `../../cli-contract.md`, `../../configuration.md`, `../../operational-trace.md`, `../../persistence.md`, `../../export-contract.md`.
- **Deliver:** all commands/outcomes/exits/paths/security/rotation/recovery/no-retry/no-mobility behavior.
- **Verify:** documented examples run in isolated sandbox.
- **Done when:** human and automation contracts match schemas.

### T194 [ ] Finalize migration, recovery, and troubleshooting documentation
- **Depends:** T173, T178–T182
- **Files:** `../../migration.md`, `../../recovery.md`, `../../troubleshooting.md`.
- **Deliver:** schema upgrade/future-version/corruption/overlap/provider failure/trace lookup/uncertain interruption guidance.
- **Verify:** every diagnostic reason has recovery path or explicit non-recoverable statement.
- **Done when:** docs do not promise replay, exact provider history, or absent side effects.

### T195 [ ] Implement one canonical quality command
- **Depends:** T019–T030, T167–T194
- **Files:** `xtask/src/quality.rs`, `quality/manifest.toml`, `quality/evidence/v1/quality-report.schema.json`, `xtask/tests/quality.rs`.
- **Deliver:** finalize T028 incremental manifest with docs, format, check, Clippy, architecture, schemas, provider conformance, migrations, all E2Es, closure, reference, dependency/advisory/license, semantic judge.
- **Verify:** clean checkout command and fail-fast/report behavior.
- **Done when:** hooks and CI invoke this implementation rather than duplicates.

### T196 [ ] Set and enforce suite runtime budget/sharding
- **Depends:** T195
- **Files:** `quality/shards.toml`, `../../testing.md`, `.github/workflows/quality.yml`, `xtask/src/quality.rs`, `xtask/tests/quality.rs`.
- **Deliver:** measured clean/repeat runtime, deterministic shards with isolated artifacts, explicit budget.
- **Verify:** each shard plus aggregate closure/reference reports.
- **Done when:** no required scenario is ignored/quarantined to meet budget.

### T197 [ ] Run repeated flake and process-leak audit
- **Depends:** T178–T196
- **Files:** `target/quality-artifacts/T197/**`.
- **Deliver:** repeated concurrency/trace/provider tests, orphan-process check, leftover-file/lock check, seed stability, and T197 publication candidate.
- **Verify:** 20 consecutive targeted runs plus 5 complete sharded-gate runs on each supported OS; completion report supplies exact commands/evidence and stops. Orchestrator creates T197 commit, runs T028 range gate, and publishes candidate branch before T198.
- **Done when:** no timing-only ordering/leak remains and remote candidate has determinate per-commit judge/quality results.
- **Stop:** any defect requires new exact corrective-task Files before tracked edit; rerun full audit afterward.

### T198 [ ] Orchestrator-only: configure repository protection and required CI
- **Depends:** T029, T195–T197
- **Files:** `../../development-policy.md`, `quality/repository/{before,after,rollback}.json`, `quality/repository/README.md`.
- **Deliver:** orchestrator exports current branch-protection/ruleset state, verifies exact T197 required-check name, records recovery owner, applies protected `main`/branch-current/release/bypass policy through authenticated `gh`, and provides exact restore commands from `before.json`.
- **Verify:** failing/unavailable judge or quality blocks test PR; real judge runs in Actions; rollback dry-run/schema check succeeds; controlled apply/restore/reapply proves recovery without locking owner out; commit redacted settings evidence.
- **Done when:** server authority matches policy and reversible evidence; any failed apply restores prior state before stop.
- **Stop:** hosting plan cannot enforce requirement; owner chooses alternative authority before release.

### T199 [ ] Run clean-room final acceptance audit
- **Depends:** T191–T198
- **Files:** `target/quality-artifacts/T199/<sha>/**`.
- **Deliver:** `cargo clean` then full publication gate, operation closure, 21 reference rows, all invariant/facet evidence, dependency and docs reports.
- **Verify:** exact command from clean clone with isolated user config; dependency retrieval may use documented Cargo network/cache, while product/provider scenarios remain network-disabled unless explicitly testing network policy; real judge access is provisioned.
- **Done when:** C12 report is green and no known required failure remains.
- **Stop:** any tracked fix requires separate exact corrective-task scope and new candidate commit/report.

### T200 [ ] Close initial implementation change
- **Depends:** T001–T199
- **Files:** `README.md`, `tasks.md`, `coverage.md`, `../../../README.md`, `../../../CHANGELOG.md`.
- **Deliver:** all tasks `[x]`, decisions resolved, generated evidence linked, shipped behavior/version recorded, no stale pending language.
- **Verify:** canonical publication gate on exact candidate commit and every commit in push range.
- **Done when:** owner approves closure and protected remote accepts publication.
- **Stop:** any task, invariant, operation facet, reference behavior, judge, or authoritative CI result is incomplete.
