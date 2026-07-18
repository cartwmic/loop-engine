# Initial Implementation Decision Gates

**Status:** D001–D016 resolved and frozen at C0 (2026-07-17); settlement complete via T001–T016.

These implementation-stage decision gates are owner-accepted and frozen in foundation/contract documentation. Phase 0 resolved them before dependent code begins. Recommendations favored the smallest design that satisfies settled invariants.

A decision is resolved only when:

- owner accepts one option;
- affected foundation/contract docs are updated in same commit;
- dependent task contracts are amended if paths or schemas change;
- parent-rubric semantic judge passes before publication.

## Decision index

| ID | Decision | Recommendation | Blocks |
|---|---|---|---|
| D001 | Runtime packaging and persistence | Accept C1–C4: native CLI, bundled SQLite, exact-revision gate, Rust `xtask` | Most implementation |
| D002 | Initial platform support | macOS and Linux; defer Windows | paths, process control, permissions, CI |
| D003 | License | Dual MIT/Apache-2.0 | manifests, release docs, `deny.toml` |
| D004 | Application operation catalog | 21-operation catalog below | operations, CLI, E2E closure |
| D005 | Provider protocol v1 | fresh process, one JSON request/result, five tagged roles | provider integration and schemas |
| D006 | Structured CLI contract | schema v1; exit 0/2/1 and usage 64 | renderers, harness, compatibility |
| D007 | Configuration and filesystem layout | OS user dirs plus `LOOP_ENGINE_HOME`; TOML defaults only | startup, tests, persistence, traces |
| D008 | Resource bounds and timeouts | Named bounds table, pagination, cursor v1, trace reservations; frozen in cli-contract.md | model, protocol, tracing, E2Es |
| D009 | SQLite schema/concurrency policy | WAL, foreign keys, busy timeout, short writes, forward migrations | persistence and overlap tests |
| D010 | Operational trace schema and post-init failure behavior | JSONL v1; fail before dispatch only on init; report later sink failures without lying about commit | startup, trace, outcomes |
| D011 | Journal granularity | one aggregate entry per meaningful operation/attempt | schema, history, atomicity |
| D012 | Semantic judge provisioning | generic JSON executable contract; real configured judge required for publication | every future push |
| D013 | Provider fixtures | Rust executable fixtures outside product crates | protocol and E2Es |
| D014 | Canonical graph encoding | canonical integration DTO sorted by semantic identity; SHA-256 | run creation and persistence |
| D015 | Audit export scope | include `run.export` producing state JSON and journal JSONL | catalog, schemas, E2Es |
| D016 | Project defaults discovery | nearest ancestor `.loop-engine.toml`, defaults only | configuration tests and docs |

## D001 — Runtime packaging and persistence

**Status:** Resolved by T001 (2026-07-17).

**Foundation status:** C1–C4 are settled in [invariants.md](../../invariants.md); Rust/synchronous/three-crate direction remains settled.

**Owner decision:** Accept recommendation below. Exact toolchain, dependency versions/features, workspace fixture exclusion, and distribution policy are frozen in [technology.md](../../technology.md).

**Recommended decision:**

- one native `loop-engine` control-plane executable;
- bundled SQLite through `rusqlite`;
- exact-revision local/publication gate;
- non-shipping Rust `xtask` for one canonical implementation of gates/hooks;
- initial package version `0.1.0`, source/Cargo-built native binary only for MVP, no installer/package-manager commitment, and no public MSRV promise before a stable release;
- T001 freezes exact toolchain plus crate-by-crate dependency versions/features from `docs/technology.md`; T018/T019 preload complete product/xtask manifests and lockfile; scenario/reference providers are root-excluded standalone packages with tracked lockfiles.

**Why:** This turns existing strong technology direction into one coherent minimal stack. It avoids daemon, external database, ORM, dual authorities, and duplicated shell logic.

**Required documentation updates:** `../../invariants.md`, `../../technology.md`, `../../testing.md`, `../../architecture.md`.

**Stop condition:** Do not create Cargo manifests or migration schema until accepted. *(Unblocked by T001.)*

## D002 — Initial platform support

**Status:** Resolved by T002 (2026-07-17).

**Owner decision:** Accept recommendation below. Exact OS/architecture matrix, CI scope, process termination, permission semantics, path semantics, and unsupported-platform policy are frozen in [technology.md](../../technology.md); [testing.md](../../testing.md) mirrors testing-specific scope only.

**Recommended decision:** macOS and Linux for MVP. Defer Windows until a concrete need exists.

**Why:** Current-user-only file modes, process-group timeout termination, Unix signals, shell fixture behavior, and path semantics can be implemented and tested without speculative platform abstraction.

**Required documentation updates:** `../../technology.md`, `../../testing.md`.

**Stop condition:** Windows inclusion requires redesign of D005, D007, D010, D013, and their tests before code. *(Unblocked for macOS/Linux.)*

## D003 — License

**Status:** Resolved by T003 (2026-07-17).

**Owner decision:** Dual MIT/Apache-2.0.

**Required outputs:** root `LICENSE-MIT` and `LICENSE-APACHE` with canonical license texts; root `README.md` dual-license notice; T017 package metadata policy below; dependency-license allowlist unblocked for T030.

**T017 package metadata policy:**

- workspace and every shipping crate use SPDX expression `license = "MIT OR Apache-2.0"`;
- workspace package metadata references root `readme = "README.md"` where applicable;
- canonical license texts live only in root `LICENSE-MIT` and `LICENSE-APACHE`; manifests do not embed full license bodies;
- root-excluded `test-support/providers/*` standalone fixtures use the same SPDX expression unless a fixture documents a divergent test-only policy.

**Why:** Dual licensing matches common Rust ecosystem practice, preserves the Apache-2.0 patent grant, and keeps dependency policy compatible with typical permissive crate licenses.

**Stop condition:** Dependency/license gate and release metadata cannot complete without owner choice. *(Unblocked by dual MIT/Apache-2.0.)*

## D004 — Application operation catalog

**Status:** Resolved by T004 (2026-07-17).

**Owner decision:** Accept the 21-operation catalog below. Canonical argv, facets, reason taxonomy, lifecycle ownership, and verification rules are frozen in [operation-catalog.md](../../operation-catalog.md).

**Recommended final catalog:**

```text
provider.add
provider.list
provider.check
provider.update
provider.rename
provider.disable
provider.restore
run.create
run.list
run.show
run.graph
run.history
run.evidence.add
run.evidence.list
run.annotate
run.label
run.request
run.guidance
run.compatibility
run.terminate
run.export
```

Rationale:

- `provider.update` is required by provider-drift storyboards but absent from one architecture list.
- `provider.check` covers protocol conformance/current emitted-graph findings for one registration and byte/count-paged `--active-runs` compatibility across stored active graph snapshots without per-run journal fan-out. Each invocation resolves registration once, spends one of ten call slots on current `describe`/conformance, then processes at most nine active-run compatibility rows in stable keyset order. It completes for zero rows or findings and errors whole page on process/protocol/evaluation failure; cursor retrieves next page.
- `run.compatibility` checks one active stored run/capability contract and remains non-latching; after run lookup it atomically appends compatibility-attempt/provider-observation journal facts, including drift, without changing state/version. Registration-wide `provider.check --active-runs` retains explicit no-fan-out exemption.
- `run.export` resolves testing requirements and the read-only export allowance in [export-contract.md](../../export-contract.md) without import/restore.
- `provider.list --active-runs-for <registration-id>` exposes byte/count-paged impact IDs. `provider.update` completes with affected count plus link, never unbounded IDs. `provider.disable` with active runs enters non-mutating warning pagination: initial call and `--warning-cursor` pages name affected runs, but only final page emits opaque `ack_token` bound to registration/config revision/full active-set digest. `--allow-active-runs <ack_token>` authorizes tombstone; first/intermediate cursors or independently supplied digest reject. Atomic mutation rechecks token binding and current active set.
- Provider add/update/rename/disable/restore are authoritative catalog mutations, not run-state/run-journal mutations. Fresh `provider.list` verifies successful or unchanged catalog state; no per-run journal entry is created, matching I40. Invocation trace remains diagnostic proof of attempted outcome.
- `--list-operations`, help, version, schema generation, and internal provider roles are driver/tooling functions, not application operations.

Provider handles use lowercase ASCII with no normalization. ABNF: `handle = alnum / (alnum *126(handle-mid) alnum)`; `handle-mid = alnum / "." / "_" / "-"`; `alnum = %x30-39 / %x61-7A`. One-character handles are valid. Handles are case-sensitive conveniences, never identity.

**Required outputs:** stable ID table, exact command/argv/flag mapping, handle grammar, facet flags, complete stable reason taxonomy, registration-wide versus per-run compatibility ownership, and explicit exclusions.

**Stop condition:** Do not implement catalog closure until all 21 IDs and command ownership are accepted.

## D005 — Provider protocol v1

**Status:** Resolved by T005 (2026-07-17).

**Owner decision:** Accept recommendation below. Process lifetime, stdin/stdout framing, version negotiation, five roles, role-specific result matrix, environment policy, resolve-once registration handoff, unknown-field policy, timeout/termination, and no-state-authority rule are frozen in [provider-protocol-v1.md](../../provider-protocol-v1.md).

**Recommended transport:**

- one fresh provider process per provider operation;
- literal executable and argument vector; no shell interpolation;
- explicit working directory independent of caller CWD;
- provider inherits caller environment unchanged; registration stores no environment overrides; inherited environment is never emitted into trace payloads;
- core operation resolves current registration exactly once through provider-catalog capability and passes immutable resolved config to invoker; invoker never queries catalog;
- one UTF-8 JSON request on stdin, then EOF;
- exactly one UTF-8 JSON result on stdout;
- bounded stderr as diagnostic stream;
- protocol-major and tagged operation/result in envelopes;
- unknown fields ignored within major; new same-major fields optional;
- unsupported major errors;
- no retries;
- timeout kills provider process group on supported Unix platforms.

**Provider roles:**

```text
describe
validate_inputs
evaluate_gates
live_guidance
check_compatibility
```

**Semantic constraints:**

- `describe` receives no candidate values.
- `validate_inputs` cannot return topology.
- `evaluate_gates` returns exactly complete verdicts with optional valid evidence, explicit incompatibility, or evaluation error.
- `live_guidance` returns exactly advisory guidance, explicit stored-guidance incompatibility, or evaluation error; it returns no evidence/state mutation.
- `check_compatibility` returns non-latching capability findings.
- each role has explicit valid/denial/finding/incompatibility/evaluation-error applicability; roles must not invent generic rejection variants.
- no result may set engine state or target transition.

| Role | Role-valid semantic results |
|---|---|
| `describe` | completed description only; role has no denial variant; process/protocol inability errors; consumer maps semantically invalid graph to creation error or conformance finding |
| `validate_inputs` | accepted values, rejected values, or evaluation error |
| `evaluate_gates` | complete verdict set, explicit incompatibility, or evaluation error |
| `live_guidance` | advisory guidance, explicit stored-guidance incompatibility, or evaluation error |
| `check_compatibility` | completed capability findings, including incompatibility, or evaluation error |

Transport/process/protocol failures remain operation errors for every invoked role. Engine outcome mapping follows D004 reason taxonomy; an incompatible compatibility report completes rather than rejects.

**Required documentation updates:** `../../provider-protocol-v1.md`, `../../technology.md`, `../../architecture.md`.

**Stop condition:** Do not persist graph snapshots or publish provider examples before schemas and same-major policy freeze. *(Transport and same-major policy unblocked by T005; canonical graph encoding unblocked by T014; evidence wire schemas remain T084.)*

## D006 — Structured CLI contract

**Status:** Resolved by T006 (2026-07-17).

**Owner decision:** Accept recommendation below. Schema v1, additive/breaking/support rule, global flags, exact 21-operation argv copy, top-level envelope fields, run summary, evidence-recorded status, reason shape, stdout/stderr/trace boundaries, human parity table, exit `0`/`2`/`1`/`64`, and contract examples are frozen in [cli-contract.md](../../cli-contract.md).

**Frozen schema:** version `1` with stable top-level fields:

```text
schema_version
operation
request_id
trace
outcome
reason
data
diagnostics
```

Run-related `data` includes `run` (lifecycle, current state, `state_changed`), `requestable_events`, and `evidence_recorded` when relevant.

**Frozen exit behavior:**

- `0` completed;
- `2` domain rejection;
- `1` operation error;
- `64` pre-dispatch usage/configuration/platform/parse failure.

Structured mode emits exactly one JSON outcome envelope on stdout after dispatch. Pre-dispatch failures use rich stderr (JSON object when `--format json`). Trace remains separate file. Provider streams never bypass engine output.

Schema v1 additions are backward-compatible only when fields are optional and unknown fields can be ignored. Removing, renaming, changing meaning/type, or making a field newly required needs a new schema version. MVP supports only current schema version and makes no support-duration promise for superseded versions. D015 applies same rule to export schemas.

**Required documentation updates:** `../../cli-contract.md`, `../../ux-storyboards.md`, `../../testing.md`.

**Stop condition:** Do not build CLI harness or public operation route before schema, compatibility, argv, and exit behavior freeze. *(Unblocked by T006.)*

## D007 — Configuration and filesystem layout

**Status:** Resolved by T007 (2026-07-17).

**Owner decision:** Accept recommendation below. Frozen in [configuration.md](../../configuration.md).

**Selected layout:**

- empty `LOOP_ENGINE_HOME` is treated as unset and selects OS default layout (not a configuration error);
- `LOOP_ENGINE_HOME` overrides all machine-local roots for tests and portable local use; when set to a non-empty value, `config.toml`, `state.db`, and `traces/` live directly under the resolved machine home root;
- otherwise Linux uses XDG config/state directories (`~/.config/loop-engine/config.toml`, `~/.local/state/loop-engine/state.db`, `…/traces/`) and macOS uses `~/Library/Application Support/loop-engine/` for the same three names;
- exactly one SQLite database file `state.db` under the machine state root;
- exactly one trace directory `traces/` under the machine state root;
- global `config.toml` under the user config root;
- optional project `.loop-engine.toml` contains defaults/references only; [ancestor discovery](../../configuration.md#project-default-discovery) is frozen in configuration.md (D016, T016).

**Precedence:** CLI flags > project defaults > global defaults > built-in defaults.

**Selected policies:**

- TOML `schema_version = 1` with `[defaults]` keys `format`, `provider`, `timeout_seconds` only; parse with workspace `toml` `1.1.3`;
- unknown keys rejected at load time with actionable diagnostics;
- forbidden provider-executable/persistence-override keys rejected in both global and project files;
- lexical normalization expands `~`, resolves relative registration paths against caller CWD at mutation time only, and stores absolute paths without symlink canonicalization; relative `LOOP_ENGINE_HOME` values are invalid and never resolved against caller CWD;
- `LOOP_ENGINE_HOME` machine home root: when the lexically normalized path exists, resolve symlinks on that path component only; when it does not exist, use the lexical absolute path as identity and permit first-use directory creation without claiming symlink canonicalization;
- registration mutations accept nonexistent executable/working-directory paths; provider-dependent invocation errors when paths are missing or unusable;
- configured-spelling identity stores lexical-normalized absolute paths and does not rewrite to symlink targets;
- malformed/oversize/unsupported-version configuration fails pre-dispatch with exit `64` and `phase = "config"`.

Project configuration cannot define or redefine provider registration selected by existing run.

**Stop condition:** Do not implement path-dependent persistence, tracing, or E2E isolation before exact names and precedence freeze. *(Unblocked by T007.)*

## D008 — Resource bounds and timeout defaults

**Status:** Resolved by T008 (2026-07-17).

**Owner decision:** Accept recommendation below. Named scalar/payload/path/argv/config/diagnostic/trace/timeout bounds, count+byte keyset pagination, cursor v1 schema, operational-trace reservation arithmetic, and evidence no-truncation policy are frozen in [cli-contract.md](../../cli-contract.md) § [Resource bounds (D008)](../../cli-contract.md#resource-bounds-d008) and § [Collection pagination and cursor v1](../../cli-contract.md#collection-pagination-and-cursor-v1).

**Policy (summary):**

- One canonical named bounds table; no magic numbers elsewhere.
- Component budgets are subordinate to aggregate envelopes (gate request, describe result, journal entry, structured CLI envelope).
- Caller-owned overflow rejects before provider; provider-owned malformed/oversized results error; **selected evidence is never truncated**.
- Captured provider stderr may use explicit truncation markers in trace only when protocol stdout remains independently complete.
- Every growing collection/report is count- and byte-paged; `--limit` is a count ceiling; encoder stops before the page data budget and never truncates a record.
- Provider-invoking pages stop before the eleventh provider call; trace reservations are cross-process with atomic reserve-to-actual conversion.
- Pagination uses opaque URL-safe-base64 cursor v1 with collection name, normalized filter fingerprint, and last key.
- Evidence locators are bounded opaque UTF-8; engine does not dereference or judge portability.

**Required documentation updates:** `../../cli-contract.md`, `../../provider-protocol-v1.md`, `../../configuration.md`, `../../technology.md`.

**Stop condition:** Bounds and collection pagination must be represented once in typed configuration/contracts and referenced by schemas/tests, not duplicated as magic numbers. *(Unblocked by T008.)*

## D009 — SQLite schema and concurrency policy

**Status:** Resolved by T009 (2026-07-17).

**Owner decision:** Accept recommendation below. Connection pragmas, busy/locking policy, migration and schema-version compatibility, transaction boundaries, workflow and registration-config CAS, catalog-mutation affected-run digest guard, create-versus-catalog writer linearization, label/note/evidence concurrency, and rollback narratives are frozen in [persistence.md](../../persistence.md).

**Selected policy (summary):**

- bundled SQLite via `rusqlite`; `foreign_keys=ON`, `journal_mode=WAL`, `synchronous=FULL`, `busy_timeout` from `sqlite_busy_timeout_ms` (T008), `temp_store=MEMORY`;
- forward-only transactional migrations through `rusqlite_migration`; refuse newer unsupported schema; no downgrade or dual-write authority;
- no write transaction or write lock held across provider invocation; snapshot load and provider subprocess run outside write transactions;
- post-provider CAS on `(workflow_state_version, lifecycle_version)` for gated transitions; gate-free state-changing `run.request` linearizes re-read/resolve/mutate in one write transaction with the same version CAS; stale branch journals error attempt without state change;
- every registration stores monotonically increasing `config_revision`; `run.create` resolves revision before provider work and atomically rechecks enabled status/revision before insert; mismatch yields `provider.registration.stale` with no run/journal;
- `provider.update` / `provider.disable` / `provider.restore` compute affected active-set digest (SHA-256 over sorted active run IDs) and mutate catalog in one write transaction; disable `ack_token` binds `(registration_id, config_revision, active_set_digest)`; authorized disable rechecks both;
- create-versus-catalog mutation linearizes by SQLite writer order plus semantic rechecks;
- `run.label`, `run.annotate`, and `run.evidence.add` do not increment workflow/lifecycle versions; `run.evidence.add` always appends journal in the same transaction;
- post-lookup `run.guidance` and `run.compatibility` rejections/errors, including terminal denial, append attempt journal when persistence is available;
- commit I/O failure requires fresh-connection authoritative verification; outcomes never falsely claim rollback or mutation;
- expected `SQLITE_CONSTRAINT` failures map to frozen domain reason codes; unexpected integrity failures map to `persistence.failed`;
- per-run `journal.sequence` allocated atomically with each insert;
- atomicity E2Es inject SQLite abort triggers/fixture corruption through test harness setup only; production code receives no fault-injection branch.

Schema stores query-critical authority in columns and bounded versioned snapshots/facts in integration-owned JSON DTOs.

**Required documentation updates:** `../../persistence.md`, `../../technology.md`, `../../architecture.md`, `../../testing.md`.

**Stop condition:** Migration `0001` must include stale-evaluation attempt branch and evidence associations before any persisted run exists. *(Unblocked by T009.)*

## D010 — Operational trace contract

**Status:** Resolved by T010 (2026-07-17).

**Owner decision:** Accept recommendation below. JSONL v1 categories, fields, permissions, budget behavior, driver/parse behavior, late sink-failure truthfulness, and Unix `SIGXFSZ`/`RLIMIT_FSIZE` E2E contract are frozen in [operational-trace.md](../../operational-trace.md).

**Recommended schema:** JSONL event schema version `1`, one request ID and file per CLI invocation.

Required event categories:

- invocation/dispatcher start and finish/error;
- provider start and finish/failure with all bounded payloads and streams;
- persistence intent, version check, commit, rollback;
- targeted transition/gate/lifecycle/compatibility/stale decisions.

Unix defaults: trace directory mode `0700`, file mode `0600`. Rotation uses count and total-byte caps, coordinates concurrent processes, and never removes an open trace.

**Post-initialization write failure:** trace initialization failure blocks dispatch. Later trace-write failure cannot roll back a committed state change; preserve true operation outcome and attach diagnostic when possible rather than misreporting commit status.

**Deterministic Unix E2E injection:** external wrapper ignores `SIGXFSZ` and applies `RLIMIT_FSIZE`; production binary receives no test branch. Run two cases: (1) provider-dependent read/report with limit above initialization but below provider events proves late `EFBIG`, truthful report, and cleanup; (2) provider-free durable annotation with limit above initialization/start but below persistence/outcome events proves operation continues, committed annotation/history survive fresh-process read, and envelope never reports rollback/error for committed state. Diagnostic is asserted when observable on macOS/Linux.

**Literal invocation scope:** help, version, and argument errors are invocations and must initialize a trace unless owner changes I46 explicitly.

**Required documentation updates:** `../../operational-trace.md`, `../../technology.md`, `../../testing.md`, `../../ux-storyboards.md`.

**Stop condition:** Startup ordering and late sink-failure behavior must freeze before CLI main exists. *(Unblocked by T010.)*

## D011 — Journal granularity

**Status:** Resolved by T011 (2026-07-17).

**Owner decision:** Accept recommendation below. Immutable entry kinds, attempt shape, monotonic `sequence`, correction links, provider/gate/evidence nesting, `state_changed`/self-loop alignment, component and aggregate encoded-size bounds, oversize rejection, and operation journal obligations are frozen in [journal-contract.md](../../journal-contract.md).

**Recommended model:** one aggregate immutable journal entry for each meaningful operation or event/guidance attempt, with nested provider facts, gate verdicts, evidence associations, state before/after, reason, outcome, actor metadata, and note.

This satisfies explainability without event-sourcing micro-events. Trace remains fine-grained operational diagnostics and does not replace journal authority.

**Selected policy (summary):**

- eight immutable `entry_kind` values covering every per-run journal obligation;
- per-run monotonic `sequence` allocated atomically with insert; `corrects_sequence` links clarifications without editing prior rows;
- `transition.applied` records completed self-loops while CLI `state_changed` remains `false` when the workflow state identifier is unchanged;
- nested `provider_facts`, `gate_verdict_facts`, and `evidence_associations` bounded by canonical D008 names;
- `note` field on annotation/transition/termination attempts; `annotation` entry kind for `run.annotate`;
- pre-insert `encoded_len` check against `journal_entry_encoded_bytes`; one-byte-over rejects with no truncation;
- provider-catalog and read operations append no per-run journal; rejected/errored `run.create` appends none.

**Required correction model:** optional link from new correction/clarification entry to prior entry; no edit/delete.

**Required documentation updates:** `../../journal-contract.md`, `../../architecture.md`, `../../ux-storyboards.md`.

**Stop condition:** Do not design tables or history rendering until entry kinds and minimum fields freeze. *(Unblocked by T011.)*

## D012 — Semantic judge provisioning

**Status:** Resolved by T012 (2026-07-17).

**Recommended contract:** generic executable receives versioned JSON request and emits one versioned JSON response.

Request includes:

- mode: local or publication;
- parent and candidate revision IDs;
- exact diff;
- resulting relevant docs;
- parent-versioned rubric content;
- deterministic command evidence.

Response is `pass`, `fail`, or `indeterminate` with cited rubric rules and changed/resulting lines. Invocation unavailable/timeout/malformed is `unavailable`.

Policy:

- bootstrap exception was consumed by foundation commit/publication `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`; no later commit receives exception;
- before focused rubric files exist, real judge treats that committed foundation's I47, testing Git-enforcement section, tenets, and architecture rules as seed parent rubric under T012 manifest; T025 focused rubrics apply only to following commit;
- no implementation push occurs before T029, then every commit in first post-foundation range must have determinate report;
- local determinate fail blocks;
- local unavailable/indeterminate warns;
- publication fail/unavailable/indeterminate blocks;
- candidate rubric cannot judge itself;
- fixture judge may test protocol but must never satisfy real publication.

**Owner must provision:** actual judge executable/configuration for local publication checks during T012, seed-foundation rubric manifest, plus named GitHub Actions secret/config owner for T029. T012 requires real local smoke pass against foundation-parent candidate, not only a plan. Product runtime must not depend on it.

**T012 outputs:**

- contract: [quality/semantic-judge/v1/README.md](../../../quality/semantic-judge/v1/README.md);
- executable: `quality/semantic-judge/v1/judge` with `openai-codex/gpt-5.6-sol` via `pi` (`quality/semantic-judge/v1/config.json`);
- foundation seed rubric: [quality/rubrics/manifest.json](../../../quality/rubrics/manifest.json) and `quality/rubrics/foundation-seed.v1.md`;
- policy/commands: [development-policy.md](../../development-policy.md);
- T029 provisioning owner: repository variable `LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE` and secret `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON`.

**Required documentation updates:** `../../development-policy.md`, `../../testing.md`, `../../architecture.md`.

**Stop condition:** C0 cannot close and no post-bootstrap commit may be pushed until every commit in range gets determinate pass from real configured judge. *(Contract frozen by T012; publication smoke required before C0 close.)*

## D013 — Provider fixtures

**Status:** Resolved by T013 (2026-07-17).

**Owner decision:** Accept recommendation below. Fixture language/runtime, package location, subprocess isolation, invocation ledger, and process-failure helper policy are frozen in [technology.md](../../technology.md) and [testing.md](../../testing.md).

**Recommended decision:** Rust fixture executables in non-product test-support packages.

Use:

- generic scenario provider with configurable graph/results/process behavior;
- reference software-change provider with provider-owned semantic tests;
- tiny Unix process helpers only where signal/process-group behavior requires them.

**Selected policy (summary):**

- **Language/runtime:** Rust edition `2024` with pinned toolchain `1.95.0` (D001). Each fixture package builds a native executable for the host CI triple. Core acceptance requires no Python, Node, or shell interpreter beyond what an explicit shebang-provider scenario exercises.
- **Package location:** root-excluded standalone packages at `test-support/providers/scenario-provider/` (generic configurable provider) and `test-support/providers/reference-provider/` (software-change reference graph per [reference-workflow.md](../../reference-workflow.md)). Each carries its own tracked `Cargo.lock`, is built with `cargo build --manifest-path … --locked` (or `cargo test --manifest-path … --locked`), and uses only the T001 fixture dependency set (`serde`, `serde_json`, `schemars`, `thiserror`). No workspace member or product crate may depend on either package.
- **Subprocess isolation:** fixtures are real external provider executables invoked through D005 transport. They **MUST NOT** depend on, import, link, or `include!` any product crate or generated product schema. They **MUST NOT** open, read, or write authoritative `state.db`, engine trace directories, or registration catalog. Software-domain semantics (reference workflow states/events/gates/evidence conventions) live only in `reference-provider`; core and integrations see only protocol envelopes.
- **Invocation ledger:** `scenario-provider` maintains an append-only JSONL ledger under scenario-controlled paths recording each subprocess invocation (`invocation_id`, `role`, `executable`, `argv`, `working_directory`, optional digest-mode facts). E2E harnesses read the ledger to prove invocation counts, ordering, and empty-ledger facets (for example provider-free operations). The ledger is test observability only and is never engine authority.
- **Barrier:** `scenario-provider` exposes filesystem/pipe barrier primitives so concurrency E2Es synchronize explicit overlap without timing-only sleeps.
- **Process-failure modes:** `scenario-provider` selects deterministic transport/process failure modes enumerated in [provider-protocol-v1.md](../../provider-protocol-v1.md) § Transport and process failures (malformed JSON, missing/extra stdout, unsupported major, invalid UTF-8, oversized streams, nonzero exit, signal, timeout, drift between paired creation calls, and role-specific semantic errors). Golden vectors live under each fixture package `fixtures/` tree.
- **Process-failure helpers:** when signal, process-group, or PGID behavior cannot be exercised safely from the fixture process tree alone, tiny standalone Unix executables under `test-support/providers/process-helpers/` may be invoked as the configured provider executable or child process. Helpers implement no protocol roles, import no product crates, and touch no authoritative database.

**Why:** avoids requiring Python/Node for core acceptance while preserving language-neutral protocol; keeps software concepts outside core; fixtures remain true subprocess providers exercised by production CLI E2Es.

**Required documentation updates:** `../../technology.md`, `../../testing.md`.

**Stop condition:** Fixtures must run as real subprocesses and must not import product internals, access database, or act as mocks. *(Unblocked for T135–T141 bootstrap.)*

## D014 — Canonical graph encoding

**Status:** Resolved by T014 (2026-07-17).

**Owner decision:** Accept recommendation below. Semantic ordering, included fields, canonical integration DTO v1, byte encoding, metadata treatment, golden vectors, field-change matrix, and `graph_revision` distinction from `executable_digest` and raw provider JSON are frozen in [graph-projection.md](../../graph-projection.md).

**Recommended decision:** map provider DTO into validated core graph, then map core graph into one integration-owned canonical DTO. Sort semantically unordered collections by stable ID, preserve ordered/text/metadata values exactly, serialize deterministically, and hash bytes with SHA-256.

Digest includes topology, final markers, transition gates, input declarations, state guidance, provider-facing metadata retained in snapshot, and live-guidance capability. Executable digest is separate.

**Required documentation updates:** `../../graph-projection.md`, `../../provider-protocol-v1.md`, `../../technology.md`.

**Stop condition:** Persist no graph snapshot until canonical bytes and golden vectors freeze. *(Unblocked by T014.)*

## D015 — Audit export scope

**Status:** Resolved by T015 (2026-07-17).

**Owner decision:** Accept recommendation below. `run.export` ownership, output-directory collision/permission/atomic-publication behavior, `manifest.json` / `state.json` / `journal.jsonl` schemas, deterministic ordering, manifest file hashes, evidence inventory and journal provider observations, D006 additive/breaking compatibility with no superseded-version support duration, structured CLI success shape, and no-import guarantee are frozen in [export-contract.md](../../export-contract.md).

**Recommended decision:** include `run.export` in MVP. It writes versioned `manifest.json`, `state.json`, and ordered `journal.jsonl` to an explicit new/empty output directory from one consistent read snapshot. Export schema follows D006 additive/breaking version rules; MVP supports current version only and promises no superseded-version duration.

Export:

- is read-only;
- never imports/restores/rebinds;
- never becomes write authority;
- does not promise replay;
- does not dereference external evidence locators;
- includes evidence metadata and journal provider/gate/evidence observations needed for inspection.

**Required documentation updates:** `../../export-contract.md`, `../../operation-catalog.md`, `../../testing.md`, `../../technology.md`.

**Stop condition:** Define schema versioning, overwrite behavior, ordering, and structured CLI response before implementation. *(Unblocked by T015.)*

## D016 — Project-default discovery

**Status:** Resolved by T016 (2026-07-17).

**Owner decision:** Accept recommendation below. Ancestor-search algorithm, filename, stop boundary, allowed keys, precedence, symlink/permission/error behavior, and explicit distinction from provider/executable/workflow discovery are frozen in [configuration.md](../../configuration.md) § [Project-default discovery](../../configuration.md#project-default-discovery).

**Selected policy (summary):**

- **Filename:** exactly `.loop-engine.toml` on the ancestor chain from caller CWD.
- **Search:** at configuration-load time, obtain caller CWD as an absolute path; walk parent directories upward; select the first (nearest) existing readable regular file, or symlink whose final target is a readable regular file, with that name; stop the walk at filesystem root (`/` on supported Unix platforms).
- **Stop boundary:** filesystem root only. No search above root and no coupling to `LOOP_ENGINE_HOME`, repository boundaries, or VCS metadata.
- **Allowed keys:** identical to global TOML schema v1 — `schema_version` and `[defaults]` with `format`, `provider`, `timeout_seconds` only; unknown and forbidden keys rejected at load.
- **Precedence:** CLI flags > discovered project file > global `config.toml` > built-in defaults. Project layer cannot alter machine-local roots, `state.db`, trace directory, or stored registration executable/argv/working-directory bindings (I40, I41).
- **Not provider/executable/workflow discovery:** engine **MUST NOT** scan for provider executables, import registration manifests, infer workflows from repository layout, or treat project TOML as workflow/provider authoring source (I40).
- **Symlinks:** existence checks follow symlinks to the target file; broken symlinks are skipped and search continues upward; ancestor traversal uses the absolute caller-CWD spelling and lexical `dirname` without requiring canonicalization of each ancestor component.
- **Permissions/errors:** unreadable caller CWD, untraversable ancestor, or unreadable selected project file are pre-dispatch configuration failures (`exit 64`, `phase = "config"`); no matching file on the chain is not an error — project layer is empty.

**Why:** Supplies deterministic per-directory CLI defaults without ambient provider scanning or workspace-coupled run identity.

**Required documentation updates:** `../../configuration.md`, `../../technology.md`.

**Stop condition:** Documentation must make distinction explicit before implementing ancestor search. *(Unblocked by T016.)*

## Resolution record

Fill this table as owner decisions land.

| Decision | Selected option | Date | Commit | Supersedes |
|---|---|---|---|---|
| D001 | Accept C1–C4: native `loop-engine` CLI, bundled SQLite via `rusqlite`, exact-revision local/publication gate, non-shipping Rust `xtask`; package version `0.1.0`; Cargo-built native binary only; no installer or public MSRV before stable release; root workspace excludes `test-support/providers/*` standalone fixtures | 2026-07-17 | pending orchestrator | — |
| D002 | macOS and Linux only for MVP: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`; four-target CI matrix; Unix current-user-only permissions (`0700`/`0600`); provider timeout kills process group (`SIGTERM` then `SIGKILL`); unsupported targets fail pre-dispatch with exit `64`; Windows and non-listed targets out of scope | 2026-07-17 | pending orchestrator | — |
| D003 | Dual MIT/Apache-2.0; root `LICENSE-MIT` and `LICENSE-APACHE`; README dual-license notice; workspace/crate `license = "MIT OR Apache-2.0"` for T017; dependency allowlist may permit Apache-2.0/MIT and compatible permissive licenses per T030 | 2026-07-17 | pending orchestrator | — |
| D004 | 21-operation catalog frozen in operation-catalog.md; provider.update included; registration-wide provider.check --active-runs vs per-run run.compatibility; disable ack semantics; explicit non-operations | 2026-07-17 | pending orchestrator | — |
| D005 | Fresh process per invocation; one UTF-8 JSON request on stdin then EOF; one UTF-8 JSON result on stdout; `protocol_major` 1; five roles (`describe`, `validate_inputs`, `evaluate_gates`, `live_guidance`, `check_compatibility`); role-specific result matrix; inherited environment with no registration overrides and no env in trace; resolve-once immutable `registration` handoff (invoker never queries catalog); same-major unknown-field ignore; Unix process-group timeout (`SIGTERM`, 5s grace, `SIGKILL`); no provider state authority; frozen in provider-protocol-v1.md | 2026-07-17 | pending orchestrator | — |
| D006 | Schema v1 envelope; exit 0/2/1/64; global `--format human|json`, `--help`, `--version`, `--list-operations`; exact 21-operation argv; human parity; one stdout envelope after dispatch; provider streams trace-only; frozen in cli-contract.md | 2026-07-17 | pending orchestrator | — |
| D007 | Empty `LOOP_ENGINE_HOME` treated as unset; non-empty `LOOP_ENGINE_HOME` flat override with existing-path symlink resolution or lexical identity for nonexistent roots and first-use creation; Linux XDG + macOS Application Support paths; `config.toml` / `.loop-engine.toml` schema v1; `state.db` + `traces/`; precedence CLI > project > global > built-in; reject unknown/forbidden keys; lexical absolute registration paths; nonexistent registration paths allowed at mutation; frozen in configuration.md | 2026-07-17 | pending orchestrator | — |
| D008 | Named bounds table, count+byte pagination, cursor v1, trace reservations; frozen in cli-contract.md | 2026-07-17 | pending orchestrator | — |
| D009 | Accept D009: WAL + FK + `synchronous=FULL` pragmas, `sqlite_busy_timeout_ms` busy wait (T008), forward-only migrations, post-provider and gate-free workflow CAS, registration-config CAS, affected-run digest guard, mandatory evidence journal, guidance/compatibility attempt journaling, unknown-commit verification, expected-constraint mapping, no provider-spanning write lock; frozen in persistence.md | 2026-07-17 | pending orchestrator | — |
| D010 | JSONL v1 per-invocation trace; `0700`/`0600` permissions; encoded actual+unused-reservation budget; no raw/parsed duplication; cross-process rotation; help/version/parse init; late sink failure preserves true commit outcome; external `RLIMIT_FSIZE` E2E cases; frozen in operational-trace.md | 2026-07-17 | pending orchestrator | — |
| D011 | One aggregate immutable journal entry per meaningful operation/attempt; eight frozen entry kinds; sequence + correction links; provider/gate/evidence nesting; state_changed/self-loop alignment; D008 bounds; frozen in journal-contract.md | 2026-07-17 | pending orchestrator | — |
| D012 | Generic JSON executable contract v1; real `openai-codex/gpt-5.6-sol` judge at `quality/semantic-judge/v1/judge`; foundation seed rubric manifest parent `7552af5968b4a2c10aefd01fbfa6c351817e1b8b`; no-second-bootstrap; T029 owns `LOOP_ENGINE_SEMANTIC_JUDGE_EXECUTABLE` + `LOOP_ENGINE_SEMANTIC_JUDGE_PI_AUTH_JSON`; frozen in development-policy.md | 2026-07-17 | pending orchestrator | — |
| D013 | Rust standalone executables at `test-support/providers/{scenario-provider,reference-provider}`; root workspace excluded; scenario-provider invocation ledger + barrier; process-failure modes per provider-protocol-v1; optional `test-support/providers/process-helpers/` Unix helpers for PGID/signal cases; frozen in technology.md + testing.md | 2026-07-17 | pending orchestrator | — |
| D014 | Canonical integration DTO v1; UTF-8 minified sorted-key JSON; SHA-256 `graph_revision`; golden vectors GV-01–GV-08; frozen in graph-projection.md | 2026-07-17 | pending orchestrator | — |
| D015 | Accept D015: `run.export` read-only export of `manifest.json`, `state.json`, and `journal.jsonl` to new/empty directory from one consistent snapshot; D006 schema versioning; no import/restore/replay/dereference; frozen in export-contract.md | 2026-07-17 | pending orchestrator | — |
| D016 | Nearest-ancestor `.loop-engine.toml` discovery from caller CWD; stop at filesystem root; schema v1 defaults/reference keys only; CLI > project > global > built-in; no provider/executable/workflow discovery or registration rebinding; frozen in configuration.md | 2026-07-17 | pending orchestrator | — |
