# Initial Implementation Decision Gates

**Status:** Pending owner resolution

These are implementation-stage decisions left open or merely recommended by the foundation. Phase 0 tasks must resolve them before dependent code begins. Recommendations favor smallest design that satisfies settled invariants.

A decision is resolved only when:

- owner accepts one option;
- affected foundation/contract docs are updated in same commit;
- dependent task contracts are amended if paths or schemas change;
- parent-rubric semantic judge passes before publication.

## Decision index

| ID | Decision | Recommendation | Blocks |
|---|---|---|---|
| D001 | Runtime packaging and persistence candidates | Accept C1–C4: native CLI, bundled SQLite, exact-revision gate, Rust `xtask` | Most implementation |
| D002 | Initial platform support | macOS and Linux; defer Windows | paths, process control, permissions, CI |
| D003 | License | Owner chooses MIT or MIT/Apache-2.0 before dependency policy | manifests, release docs, `deny.toml` |
| D004 | Application operation catalog | 21-operation catalog below | operations, CLI, E2E closure |
| D005 | Provider protocol v1 | fresh process, one JSON request/result, five tagged roles | provider integration and schemas |
| D006 | Structured CLI contract | schema v1; exit 0/2/1 and usage 64 | renderers, harness, compatibility |
| D007 | Configuration and filesystem layout | OS user dirs plus `LOOP_ENGINE_HOME`; TOML defaults only | startup, tests, persistence, traces |
| D008 | Resource bounds and timeouts | bounded table below; 60-second provider default | model, protocol, tracing, E2Es |
| D009 | SQLite schema/concurrency policy | WAL, foreign keys, busy timeout, short writes, forward migrations | persistence and overlap tests |
| D010 | Operational trace schema and post-init failure behavior | JSONL v1; fail before dispatch only on init; report later sink failures without lying about commit | startup, trace, outcomes |
| D011 | Journal granularity | one aggregate entry per meaningful operation/attempt | schema, history, atomicity |
| D012 | Semantic judge provisioning | generic JSON executable contract; real configured judge required for publication | every future push |
| D013 | Provider fixtures | Rust executable fixtures outside product crates | protocol and E2Es |
| D014 | Canonical graph encoding | canonical integration DTO sorted by semantic identity; SHA-256 | run creation and persistence |
| D015 | Audit export scope | include `run.export` producing state JSON and journal JSONL | catalog, schemas, E2Es |
| D016 | Project defaults discovery | nearest ancestor `.loop-engine.toml`, defaults only | configuration tests and docs |

## D001 — Runtime packaging and persistence candidates

**Foundation status:** C1–C4 are candidates; Rust/synchronous/three-crate direction is settled.

**Recommended decision:**

- one native `loop-engine` control-plane executable;
- bundled SQLite through `rusqlite`;
- exact-revision local/publication gate;
- non-shipping Rust `xtask` for one canonical implementation of gates/hooks;
- initial package version `0.1.0`, source/Cargo-built native binary only for MVP, no installer/package-manager commitment, and no public MSRV promise before a stable release;
- T001 freezes exact toolchain plus crate-by-crate dependency versions/features from `docs/technology.md`; T018/T019 preload complete product/xtask manifests and lockfile; scenario/reference providers are root-excluded standalone packages with tracked lockfiles.

**Why:** This turns existing strong technology direction into one coherent minimal stack. It avoids daemon, external database, ORM, dual authorities, and duplicated shell logic.

**Required documentation updates:** `../../invariants.md`, `../../technology.md`, `../../testing.md`, `../../architecture.md`.

**Stop condition:** Do not create Cargo manifests or migration schema until accepted.

## D002 — Initial platform support

**Recommended decision:** macOS and Linux for MVP. Defer Windows until a concrete need exists.

**Why:** Current-user-only file modes, process-group timeout termination, Unix signals, shell fixture behavior, and path semantics can be implemented and tested without speculative platform abstraction.

**Required detail:** CI matrix, supported architectures, process termination behavior, permission semantics, and unsupported-platform message.

**Stop condition:** Windows inclusion requires redesign of D005, D007, D010, D013, and their tests before code.

## D003 — License

**Owner decision required:**

- MIT; or
- dual MIT/Apache-2.0.

Do not infer this from private-repository status.

**Required outputs:** `LICENSE*`, package metadata, dependency-license allowlist, README notice.

**Stop condition:** Dependency/license gate and release metadata cannot complete without owner choice.

## D004 — Application operation catalog

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
- `run.export` resolves testing requirements and the existing optional audit-export allowance without import/restore.
- `provider.list --active-runs-for <registration-id>` exposes byte/count-paged impact IDs. `provider.update` completes with affected count plus link, never unbounded IDs. `provider.disable` with active runs enters non-mutating warning pagination: initial call and `--warning-cursor` pages name affected runs, but only final page emits opaque `ack_token` bound to registration/config revision/full active-set digest. `--allow-active-runs <ack_token>` authorizes tombstone; first/intermediate cursors or independently supplied digest reject. Atomic mutation rechecks token binding and current active set.
- Provider add/update/rename/disable/restore are authoritative catalog mutations, not run-state/run-journal mutations. Fresh `provider.list` verifies successful or unchanged catalog state; no per-run journal entry is created, matching I40. Invocation trace remains diagnostic proof of attempted outcome.
- `--list-operations`, help, version, schema generation, and internal provider roles are driver/tooling functions, not application operations.

Provider handles use lowercase ASCII with no normalization. ABNF: `handle = alnum / (alnum *126(handle-mid) alnum)`; `handle-mid = alnum / "." / "_" / "-"`; `alnum = %x30-39 / %x61-7A`. One-character handles are valid. Handles are case-sensitive conveniences, never identity.

**Required outputs:** stable ID table, exact command/argv/flag mapping, handle grammar, facet flags, complete stable reason taxonomy, registration-wide versus per-run compatibility ownership, and explicit exclusions.

**Stop condition:** Do not implement catalog closure until all 21 IDs and command ownership are accepted.

## D005 — Provider protocol v1

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

**Stop condition:** Do not persist graph snapshots or publish provider examples before schemas and same-major policy freeze.

## D006 — Structured CLI contract

**Recommended schema:** version `1` with stable top-level fields:

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

Run-related data includes lifecycle, current state, `state_changed`, requestable events, and evidence-recorded categories when relevant.

**Recommended exit behavior:**

- `0` completed;
- `2` domain rejection;
- `1` operation error;
- `64` pre-dispatch usage/configuration failure.

Structured mode emits exactly one JSON object after dispatch. Trace remains separate file. Provider streams never bypass engine output.

Schema v1 additions are backward-compatible only when fields are optional and unknown fields can be ignored. Removing, renaming, changing meaning/type, or making a field newly required needs a new schema version. MVP supports only current schema version and makes no support-duration promise for superseded versions. D015 applies same rule to export schemas.

**Stop condition:** Do not build CLI harness or public operation route before schema, compatibility, argv, and exit behavior freeze.

## D007 — Configuration and filesystem layout

**Recommended layout:**

- `LOOP_ENGINE_HOME` overrides all machine-local roots for tests and portable local use;
- otherwise use OS-appropriate user config/state directories;
- one SQLite database under machine state root;
- one trace directory under machine state root;
- global TOML defaults under user config root;
- optional project `.loop-engine.toml` contains defaults/references only.

**Precedence:** CLI flags > project defaults > global defaults > built-in defaults.

Project configuration cannot define or redefine provider registration selected by existing run. Phase T007 must also freeze unknown-key handling, symlink/lexical normalization, nonexistent executable/working-directory handling, and whether path identity preserves configured spelling. Registration executable and working directory are normalized to absolute locations at registration/update time under that selected policy.

**Stop condition:** Do not implement path-dependent persistence, tracing, or E2E isolation before exact names and precedence freeze.

## D008 — Resource bounds and timeout defaults

**Recommended initial defaults:**

| Resource | Bound |
|---|---:|
| Identifier/provider handle UTF-8 bytes | 128 |
| Optional run label UTF-8 bytes | 256 |
| Note text UTF-8 bytes | 64 KiB |
| Opaque actor metadata encoded | 16 KiB |
| Evidence locator UTF-8 bytes | 8 KiB |
| One filesystem path UTF-8 bytes | 4 KiB |
| Provider argv elements | 128 |
| One provider argument UTF-8 bytes | 16 KiB |
| Provider argv encoded total | 256 KiB |
| One TOML configuration file | 1 MiB |
| Provider request JSON | 4 MiB |
| Provider result/stdout | 1 MiB |
| Provider stderr | 1 MiB |
| Canonical graph projection | 512 KiB |
| Accepted input values encoded | 1 MiB total |
| One evidence record encoded | 64 KiB |
| Inline evidence context | 1 MiB total |
| Selected evidence context | 1 MiB total |
| Non-evidence provider snapshot/envelope | 512 KiB |
| Static/live guidance text | 256 KiB |
| One compatibility/impact finding | 64 KiB |
| Evidence associations in one journal entry encoded | 256 KiB total |
| Provider observation facts in one journal entry encoded | 256 KiB total |
| Gate/verdict facts in one journal entry encoded | 512 KiB total |
| One aggregate journal entry encoded | 2.5 MiB |
| One diagnostic | 8 KiB |
| Diagnostics per result | 100 |
| Metadata nesting | 16 levels |
| Provider timeout | 60 seconds |
| Collection page default | 100 records |
| Collection page maximum | 1,000 records |
| Collection page encoded data budget | 3 MiB |
| Structured CLI envelope | 4 MiB |
| Trace initialization/base reservation | 16 MiB |
| One provider-call trace reservation | 10 MiB |
| Provider calls in one paged invocation | 10 |
| One active trace file | 120 MiB |
| Trace retained files | 100 |
| Trace retained bytes, including open files/reservations | 128 MiB |

Component budgets are subordinate to aggregate envelopes: maximum gate request is at most 1 MiB selected + 1 MiB inline + 1 MiB inputs + 512 KiB snapshot/envelope, leaving 512 KiB framing headroom inside 4 MiB. Maximum 512 KiB graph leaves result-envelope/diagnostic headroom inside 1 MiB stdout. Journal diagnostic aggregate is at most 800 KiB; together with 256 KiB associations, 256 KiB provider facts, 512 KiB verdict facts, 64 KiB note, 16 KiB actor metadata, and framing, one encoded entry remains below 2.5 MiB and therefore fits one 3 MiB history page without truncation. Caller-owned overflow rejects before provider; provider-owned malformed/oversized result errors. Oversized authoritative request/result/evidence context errors or rejects according to caller/provider responsibility; selected evidence is never truncated. Captured provider streams may use explicit truncation markers only if protocol result remains independently complete and contract says so.

Every growing collection/report is both count- and byte-paged. `--limit` is count ceiling, never size promise; encoder stops before 3 MiB page-data budget, emits cursor to first unreturned record, and never truncates a record. Trace encoding budgets are on-disk, not raw-memory sizes. Dispatcher request and outcome are embedded once as JSON values, never JSON strings, and 16 MiB base covers two 4 MiB envelopes, persistence/decision events, JSONL framing, and worst-case escaping. Provider request is embedded once as JSON value; exact stdout/stderr bytes are each stored once as base64 (4/3 expansion); parsed result is not duplicated, only digest/size metadata. Thus 4 MiB request + two base64-expanded 1 MiB streams + 512 KiB facts/framing stays below 10 MiB. Schema tests calculate encoded bytes, including control-heavy JSON and binary streams.

Provider-invoking pages additionally stop before eleventh provider call. Rotation coordinator counts actual bytes plus only unused reservation remainder against 128 MiB. Each write atomically converts reserved capacity into actual bytes, so bytes are never double-counted. Trace initialization reserves 16 MiB after evicting eligible closed files; each provider call adds 10 MiB unused capacity before launch; unused remainder releases at close. Insufficient base reservation is trace-initialization failure; after page progress, insufficient next-call reservation ends page with cursor; before any row, it yields explicit resource-exhausted operation error with unchanged cursor. Thus concurrent open traces never exceed directory cap, every launched call can record complete bounded request/result/stdout/stderr, and one trace remains below 120 MiB (`16 + 10×10 = 116 MiB`). Persisted record bounds guarantee at least one record fits. This applies to provider/run/evidence/history lists, registration-wide active-run compatibility, and registration impact/affected-run reads and disable warning pages. Structured envelope keeps 1 MiB framing/diagnostic headroom.

Pagination uses opaque URL-safe-base64 JSON cursor v1 with collection/report name, normalized filter fingerprint, and last key. Registration/run pages order by `(created_at, stable_id)`; active-run/impact pages by run ID; one-run evidence by `(created_at, evidence_id)`; history by immutable per-run sequence. Empty cursor starts page. Malformed, wrong-version, wrong-collection, or filter-mismatched cursor rejects. Records are not deleted in MVP, so cursor carries no server-side state and no ordinary stale-cursor condition; unsupported cursor version is explicit rejection. CLI exposes `--cursor` and `--limit`; default/max come from table. Phase T008 publishes exact cursor schema in CLI contracts.

Evidence locator is bounded opaque non-empty UTF-8 with no NUL/control characters. Engine does not parse it as URI/path, dereference it, resolve against caller CWD, or judge portability. Self-contained versus provider-documented input-relative meaning belongs to caller/provider convention; engine rejects only syntax/bounds and preserves exact locator bytes.

**Stop condition:** Bounds and collection pagination must be represented once in typed configuration/contracts and referenced by schemas/tests, not duplicated as magic numbers.

## D009 — SQLite schema and concurrency policy

**Recommended policy:**

- bundled SQLite;
- foreign keys enabled;
- WAL mode;
- bounded busy timeout;
- forward-only transactional migrations;
- refuse newer unsupported schema;
- no downgrade or dual-write authority;
- no write transaction held across provider invocation;
- compare workflow-state/lifecycle version in post-provider transaction;
- every registration stores monotonically increasing config revision;
- run creation resolves `(registration_id, config_revision)` before provider work and atomically rechecks enabled status/revision while inserting run/creation journal; changed/tombstoned registration yields stale-provider-config operation error and no run/journal;
- update/disable/restore atomically compute affected active-set digest and mutate registration/revision in one write transaction; disable final-page acknowledgement token must bind matching digest/config revision; create-versus-catalog mutation linearizes by SQLite writer order;
- label/note/evidence appends do not increment workflow version;
- one per-run journal sequence allocated atomically;
- atomicity E2Es inject SQLite abort triggers/fixture corruption through test setup only; production code receives no fault-injection branch.

Schema stores query-critical authority in columns and bounded versioned snapshots/facts in integration-owned JSON DTOs.

**Stop condition:** Migration `0001` must include stale-evaluation attempt branch and evidence associations before any persisted run exists.

## D010 — Operational trace contract

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

**Stop condition:** Startup ordering and late sink-failure behavior must freeze before CLI main exists.

## D011 — Journal granularity

**Recommended model:** one aggregate immutable journal entry for each meaningful operation or event/guidance attempt, with nested provider facts, gate verdicts, evidence associations, state before/after, reason, outcome, actor metadata, and note.

This satisfies explainability without event-sourcing micro-events. Trace remains fine-grained operational diagnostics and does not replace journal authority.

**Required correction model:** optional link from new correction/clarification entry to prior entry; no edit/delete.

**Stop condition:** Do not design tables or history rendering until entry kinds and minimum fields freeze.

## D012 — Semantic judge provisioning

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

**Stop condition:** C0 cannot close and no post-bootstrap commit may be pushed until every commit in range gets determinate pass from real configured judge.

## D013 — Provider fixtures

**Recommended decision:** Rust fixture executables in non-product test-support packages.

Use:

- generic scenario provider with configurable graph/results/process behavior;
- reference software-change provider with provider-owned semantic tests;
- tiny Unix process helpers only where signal/process-group behavior requires them.

**Why:** avoids requiring Python/Node for core acceptance while preserving language-neutral protocol.

**Stop condition:** Fixtures must run as real subprocesses and must not import product internals, access database, or act as mocks.

## D014 — Canonical graph encoding

**Recommended decision:** map provider DTO into validated core graph, then map core graph into one integration-owned canonical DTO. Sort semantically unordered collections by stable ID, preserve ordered/text/metadata values exactly, serialize deterministically, and hash bytes with SHA-256.

Digest includes topology, final markers, transition gates, input declarations, state guidance, provider-facing metadata retained in snapshot, and live-guidance capability. Executable digest is separate.

**Stop condition:** Persist no graph snapshot until canonical bytes and golden vectors freeze.

## D015 — Audit export

**Recommended decision:** include `run.export` in MVP. It writes one versioned `state.json` and ordered `journal.jsonl` to explicit new/empty output directory from one consistent read snapshot. Export schema follows D006 additive/breaking version rules; MVP supports current version only and promises no superseded-version duration.

Export:

- is read-only;
- never imports/restores/rebinds;
- never becomes write authority;
- does not promise replay;
- does not dereference external evidence locators;
- includes evidence metadata/associations and provider observations needed for inspection.

**Stop condition:** Define schema versioning, overwrite behavior, ordering, and structured CLI response before implementation.

## D016 — Project-default discovery

**Recommended decision:** starting from caller CWD, search nearest ancestors for `.loop-engine.toml`; stop at filesystem root. File may select default registration ID/handle, output preferences, and default timeout but cannot contain executable registration definitions or database identity.

This is CLI-default discovery, not provider discovery.

**Stop condition:** Documentation must make distinction explicit before implementing ancestor search.

## Resolution record

Fill this table as owner decisions land.

| Decision | Selected option | Date | Commit | Supersedes |
|---|---|---|---|---|
| D001 | Pending | — | — | — |
| D002 | Pending | — | — | — |
| D003 | Pending | — | — | — |
| D004 | Pending | — | — | — |
| D005 | Pending | — | — | — |
| D006 | Pending | — | — | — |
| D007 | Pending | — | — | — |
| D008 | Pending | — | — | — |
| D009 | Pending | — | — | — |
| D010 | Pending | — | — | — |
| D011 | Pending | — | — | — |
| D012 | Pending | — | — | — |
| D013 | Pending | — | — | — |
| D014 | Pending | — | — | — |
| D015 | Pending | — | — | — |
| D016 | Pending | — | — | — |
