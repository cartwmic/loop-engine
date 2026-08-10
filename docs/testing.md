# Loop Engine Testing Doctrine

**Status:** E2E authority, facet coverage, runtime operation/trace proof, executable-provider coverage, no-mock policy, provider-fixture strategy, and macOS/Linux scope are settled.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Reference workflow](reference-workflow.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)
- [Application operation catalog](operation-catalog.md)
- [CLI contract](cli-contract.md)
- [SQLite persistence policy](persistence.md)
- [Export contract](export-contract.md)
- [Provider protocol v1](provider-protocol-v1.md)

## Authoritative claim

Tests cannot prove total absence of defects. Enforced claim is narrower:

> Every declared engine operation is reachable through a production driver, is observed executing in passing black-box scenarios against real providers and persistence, satisfies applicable behavioral facets, and preserves all known regression contracts.

CLI is the only current production driver.

## Behavioral authority

Only black-box production-driver tests satisfy behavioral acceptance. Tests invoke built CLI as separate process and observe documented outputs, exit codes, provider invocations, persisted state, and later CLI queries.

Lower-level unit, integration, adapter, or property tests cannot substitute for missing CLI coverage.

Pure core property tests are permitted as supplemental combinatorial defense. They must remain free of mocks and do not count toward operation completeness.

## Sequencing doctrine

Because only production-driver tests carry behavioral authority, implementation sequencing must produce a runnable production driver and black-box harness before pursuing breadth. A thin end-to-end slice — driver, one real provider fixture, one operation observed through the CLI with persistence and trace — precedes exhaustive per-operation depth. Depth added before the driver exists is unaccepted inventory under this doctrine, however well tested at lower levels.

## No-mock policy

Required behavioral tests use:

- production CLI binary;
- selected production SQLite transactional persistence integration;
- production executable-provider integration;
- controlled provider scripts/binaries implementing real protocol;
- production configuration and JSON protocol parsing.

Mock frameworks and mock-based behavioral tests are prohibited.

Temporary filesystems, executable provider fixtures, legacy databases, malformed protocol responses, deliberate corruption, and independent reference models are real test inputs rather than replacements for product behavior.

## Provider fixture strategy

Normative package layout, toolchain, and fixture build rules live in [technology.md](technology.md) § Standalone provider fixtures. This section states testing obligations only.

Required behavioral E2Es exercise the production CLI against real provider subprocesses per [provider-protocol-v1.md](provider-protocol-v1.md). Fixtures are external executables, not in-process mocks or product-library shims.

### Packages and roles

| Package | Role |
|---|---|
| `test-support/providers/scenario-provider` | Generic configurable provider: graph/input variants, gate/evidence/guidance/compatibility branches, transport/process failure modes, barrier, and invocation ledger |
| `test-support/providers/reference-provider` | Software-change reference workflow from [reference-workflow.md](reference-workflow.md); provider-owned semantic tests; engine sees only generic protocol data |
| `test-support/providers/process-helpers/` | Optional tiny Unix executables for signal/process-group/PGID cases that a fixture cannot safely self-inflict; no protocol roles |

Software-domain vocabulary and gate policies **MUST** remain inside fixture packages. Core, integrations, and CLI **MUST NOT** embed reference-workflow or scenario-specific semantics.

### Subprocess isolation

Fixtures **MUST NOT**:

- depend on, import, link, or `include!` any product crate or generated product schema;
- open, read, or write authoritative `state.db`, engine trace roots, or registration catalog;
- substitute for production CLI, persistence, or provider-invocation integration.

Fixtures **MAY** use scenario-controlled temporary directories, argv-selected JSON config files, and append-only ledger files under those directories only.

E2E harnesses register fixture binaries via `provider.add` using absolute paths produced by `cargo build --manifest-path test-support/providers/<package>/Cargo.toml --locked` for the supported host triple. Harnesses **MUST NOT** inject fixture code into the product process.

### Invocation ledger

`scenario-provider` records every provider subprocess invocation in append-only JSONL under a scenario-controlled path (selected through fixture argv/config). Each line records at minimum:

- `invocation_id`;
- `role`;
- `executable` and verbatim `argv`;
- `working_directory`;
- optional digest-mode or paired-call facts needed for drift proofs.

E2E assertions use the ledger to prove:

- expected invocation counts and ordering (for example exactly one `describe` per `provider.check` page, at most nine `check_compatibility` calls, empty ledger for provider-free facets);
- concurrent overlap when combined with the barrier;
- absence of hidden provider calls.

The ledger is test observability only. Operational trace and journal remain authoritative engine outputs.

### Process-failure and helper policy

`scenario-provider` must expose deterministic selectors for every transport/process failure in [provider-protocol-v1.md](provider-protocol-v1.md) § Transport and process failures without mock frameworks. Golden request/result/process vectors live under each fixture package `fixtures/` tree.

When a failure mode requires signal delivery, process-group establishment verification, or orphaned-child cleanup that the fixture cannot safely perform from its own tree, the harness may register a `test-support/providers/process-helpers/` executable instead. Helpers:

- implement no `describe`/`validate_inputs`/`evaluate_gates`/`live_guidance`/`check_compatibility` roles;
- import no product crates;
- touch no authoritative database;
- are used only in scenarios that explicitly target Unix process semantics.

Unix shell-script providers appear **only** in scenarios that explicitly exercise shebang/script executable configuration, not as a substitute for the Rust acceptance fixtures.

### Runtime prerequisites

Before fixture packages exist in the workspace tree, the following prerequisites are frozen:

| Prerequisite | Requirement |
|---|---|
| Toolchain | Rust `1.95.0` from root `rust-toolchain.toml` (same as product) |
| Host triples | Native fixture binaries for supported host triples; final acceptance records named macOS and Linux evidence |
| Build command | `cargo build --manifest-path test-support/providers/<package>/Cargo.toml --locked` (or `cargo test --manifest-path … --locked` for fixture-owned tests) |
| Extra runtimes | None for core acceptance (no Python/Node/shell interpreter requirement beyond explicit shebang scenarios) |
| Product coupling | Zero `path` or version dependency from any workspace member to fixture crates |

## Closed operation coverage

Core owns finite catalog of stable operation identifiers. Every production driver exposes operations it supports. Required E2E scenarios accumulate operations observed at runtime from structured driver outcomes.

Required equality:

```text
core operation catalog
= union of driver-supported operations
= operations observed in passing required E2Es
= operation envelopes observed in their passing trace files
```

A test declaration such as “covers run.request” is insufficient. Production driver must report operation actually executed, scenario must verify observable behavior, and its per-invocation JSONL trace must identify same operation/request ID.

## Structured outcome envelope

Frozen by [cli-contract.md](cli-contract.md). Every structured CLI outcome after dispatch includes stable operation ID and one of three semantic outcome classes:

- completed operation;
- domain rejection;
- operation error.

Domain rejection covers stored-graph/lifecycle denial, failed gate, invalid caller input/evidence selection, unsupported guidance, and explicit provider-declared capability incompatibility. Operation error covers invalid provider graph and, for provider-dependent requests, tombstoned/missing registration/executable, unsupported protocol major, creation drift, provider execution/evaluation/protocol/evidence failure, stale workflow-state/lifecycle version, and persistence failure. Successful explicit compatibility check completes with non-latching per-run/per-capability findings. Detailed reason codes preserve recovery paths without adding top-level classes. Reason codes are enumerated in [operation-catalog.md](operation-catalog.md) § Outcome and reason taxonomy.

Structured mode emits exactly one outcome envelope on stdout after dispatch. Always-on JSONL trace is written to separate file and never mixed into stdout. Stderr is reserved for rich pre-dispatch failure or inability to construct envelope. Provider stdout/stderr never appears on CLI stdout/stderr.

### Exit codes

| Exit | Class |
|---:|---|
| `0` | completed |
| `2` | rejected |
| `1` | error |
| `64` | pre-dispatch |

### Envelope v1 top-level fields

`schema_version`, `operation`, `request_id`, `trace`, `outcome`, `reason`, `data`, `diagnostics`. Run-related operations place `run`, `requestable_events`, and `evidence_recorded` under `data`. Completed operations use `"reason": null`.

### Human parity

Human-readable rendering must expose the same outcome class, reason code, run summary, requestable events, evidence-recorded status, and exit code as structured mode for the same invocation. E2E and harness parsers must not guess schema fields; they consume [cli-contract.md](cli-contract.md) examples and field table directly.

Normative example (rejected `run.request`):

```json
{
  "schema_version": 1,
  "operation": "run.request",
  "request_id": "01J...",
  "trace": "/machine-local/state/loop-engine/traces/01J....jsonl",
  "outcome": "rejected",
  "reason": {
    "code": "gate.failed",
    "message": "One or more required gates failed"
  },
  "data": {
    "run": {
      "id": "01J...",
      "label": "checkout-redesign",
      "lifecycle": "active",
      "state": "design-review",
      "state_changed": false
    },
    "requestable_events": ["approved", "changes-requested"]
  },
  "diagnostics": []
}
```

## Facet matrix

Every operation has primary valid-path E2E scenario. Additional required facets depend on behavior.

| Operation characteristic | Mandatory facets |
|---|---|
| Every operation | Valid path through production CLI, runtime operation-ID proof, correlated trace file, request/outcome payloads, and start/finish envelope |
| Run-state or run-journal mutation | Fresh-process CLI query verifies authoritative run state and journal |
| Successful creation | Fresh-process CLI query verifies new authoritative run and creation journal atomically |
| Rejected/error creation | Fresh-process CLI query verifies no run and no run journal |
| Provider-catalog mutation | Fresh-process `provider.list` verifies authoritative catalog state; no per-run journal is created per I40 |
| Rejectable run mutation after run lookup | Rejected path verifies run state unchanged and rejection journaled |
| Rejectable provider-catalog mutation | Rejected path verifies catalog unchanged and no per-run journal entry; invocation trace records outcome |
| Provider invoking | Completed role-valid result; role-defined denial, finding, incompatibility, or evaluation error when applicable; missing provider, timeout, crash, malformed protocol, invalid UTF-8, and bounded output |
| Gate driven | Complete passing/failing verdict set, explicit incompatibility, evaluation error, exact-set violation, empty-gate no-invocation, provider evidence pass/fail persistence, and malformed provider evidence |
| Read | Structured output contract plus invalid/not-found input |
| Lifecycle family | Applicable command-owned slice of: active, neutral final with domain meaning in state ID, intentional zero-final ongoing run, non-final terminate-only sink, explicit termination/note, empty terminal requestable events, no reopen, terminal evidence/annotation allowance, terminal label/event/advisory/compatibility rejection, and repeated termination rejection |
| Compatibility sensitive | Non-latching check completes with per-capability findings; supported/gate-free request remains usable and selected unsupported capability rejects |
| Provider-free under missing provider | Applicable safe read/mutation still completes or reaches its stored-policy result with registration/executable unavailable; provider ledger remains empty |
| Journal required | Fresh-process history proves exact required creation, event/guidance/per-run-compatibility attempt, provider drift observation, evidence, annotation, label, or termination fact and ordering |
| Trace provider boundary | Every provider-invoking path proves configured invocation facts, complete bounded payloads/streams, and finish/failure event |
| Trace persistence boundary | Every persistence path proves attempted transaction/read, applicable version check, and commit/rollback/read outcome |

Provider process-failure facets (missing executable, timeout, crash, nonzero exit, signal, malformed/wrong-major/invalid-UTF-8 protocol, oversized output) form one shared family proven through common adapter machinery. The full family closes exhaustively on at least one representative provider-invoking operation. Every other provider-invoking operation closes its operation-specific outcome facets — including no-mutation and no-journal proofs — plus at least one representative failure row, and may reference the shared family suite for the remaining rows ([I30](invariants.md#i30-end-to-end-depth-follows-operation-facets)). The cross-product of every failure mode against every operation is explicitly not required.

Lifecycle ownership is distributed, not repeated wholesale by every lifecycle-aware command: list/show/terminate own lifecycle visibility and terminal-state family; evidence/annotation own terminal append allowance; label/request/guidance/compatibility own their terminal rejection. Facet inventory must assign every family member to at least one exposure and each operation must close its applicable slice before exposure. Names in operation facet inventories must match this table exactly. No lower-level test can waive an assigned facet.

### Lifecycle-family owner table (normative)

| Lifecycle-family member | Owner operation(s) |
|---|---|
| Active run visibility | `run.list`, `run.show`, `run.terminate` |
| Neutral final with domain meaning in state ID | `run.list`, `run.show`, `run.terminate` |
| Intentional zero-final ongoing run | `run.list`, `run.show`, `run.terminate` |
| Non-final terminate-only sink | `run.list`, `run.show`, `run.terminate` |
| Explicit termination with optional note | `run.terminate` |
| Repeated termination rejection | `run.terminate` |
| Empty terminal requestable events | `run.list`, `run.show`, `run.terminate` |
| No reopen | `run.terminate` |
| Terminal evidence append allowance | `run.evidence.add` |
| Terminal annotation allowance | `run.annotate` |
| Terminal label change rejection | `run.label` |
| Terminal event request rejection | `run.request` |
| Terminal live guidance rejection | `run.guidance` |
| Terminal compatibility check rejection | `run.compatibility` |

Canonical copy: [operation-catalog.md](operation-catalog.md) § Lifecycle-family ownership.

## Required reference acceptance

The [reference software-change workflow](reference-workflow.md) is mandatory black-box acceptance case. Every behavior listed in its required acceptance section must have runtime-observed production CLI coverage. Tests may combine behaviors but cannot substitute lower-level proof or omit software-specific revision paths.

## Cross-operation E2E families

Operation facets are minimum coverage, not complete suite.

### Workflow semantics

Cover linear transitions, cycles, explicit review-revision paths, completed self-loops reporting unchanged state, zero/one/multiple final states, initial-final run, final-state outgoing-transition rejection, unknown events, ambiguous emitted graphs, multiple gates, and rejection preserving state.

### Provider graph creation

Cover input-free graph description, value-only input validation returning no topology, detected description/validation observed-executable drift error and interpreted-dependency limitation, valid graph emission, malformed JSON, explicit graph check completing with invalid finding and run creation treating same invalid graph as operation error, compatible same-major additions, unsupported protocol major, immutable registration-ID workflow identity, full-projection digest including guidance/input/live-guidance-capability declarations, missing initial/target state, duplicate identifiers, duplicate `(state,event)`, unsupported semantics, provider crash, static-guidance/no-guidance/live-capability declaration, and graph snapshot persistence.

### Provider drift

Cover machine-local stable registration ID, caller-CWD-independent resolution, handle rename preserving ID, disable releasing handle with HMAC-protected warning pagination and final-page `ack_token` traversal binding, explicit restore by ID with free handle, handle reuse not capturing stranded runs, registration changing executable/arguments/working directory during active run, gate-attempt locator/digest journaling, stored projection unchanged, provider honoring stored declarations or reporting incompatibility, non-latching per-capability report with mixed findings, unsupported selected request rejecting, and supported/gate-free events remaining usable.

### Stateful sequences

Cover realistic multi-command histories through separate processes, including provider registration by unique handle/stable ID, create with inspectable non-secret inputs and optional label, machine-wide active listing from another working directory, label change while active, provider-free current-work show/full-graph inspection/history/evidence inventory, stored live-guidance capability, explicitly requested live guidance plus journaling of completed/unsupported/lifecycle/error requests, independent evidence/note append, cold-session inventory with empty-default caller selection and guidance recommendations, event request with inline evidence, reject, transition, cycle, terminate with note, terminal annotation, and later continuation of active runs. Verify normal responses report current state, state-change status, and next requestable events.

### Persistence and journal

Cover restart, authoritative-state loading, journal ordering, atomicity of every mutation and attempt-only journal/evidence append, unknown-event and terminal-denial journaling after run lookup, no run journal for rejected creation, distinct committed-status reporting for inline evidence, selected associations, and provider evidence on applicable attempts, provider timeout/crash journaling, abrupt process death with explicitly limited audit guarantee, migration, corruption, export, interrupted processes, and history inspection before manual retry.

Normative migration, pragma, transaction-boundary, CAS, and rollback expectations: [persistence.md](persistence.md).

Normative export artifact set, ordering, manifest hashes, sibling-staging atomic publication, no-import guarantee, and failure cleanup: [export-contract.md](export-contract.md). Required scenarios prove `run.export` across active, final, and terminated runs; reject non-empty output directories; leave SQLite unchanged; never dereference evidence locators; validate exported `manifest.json` digests against on-disk payload bytes; and prove crash/fault behavior for the publication protocol.

Do not require replay or historical state reconstruction.

### Mutation safety

Verify caller-facing commands require no revision token, claim, lease, or idempotency key. Accidental overlap must not corrupt state/journal; provider verdict evaluated against stale workflow-state/lifecycle version must error without transition, while concurrent label/note/evidence append does not invalidate it. Intentional concurrent same-run collaboration remains outside accepted workflow.

Required concurrency proofs ([persistence.md](persistence.md)):

| Scenario | Both writer orders | Expected outcome |
|---|---|---|
| `run.create` vs `provider.update` / `disable` / `restore` | catalog commits first; create commits first | stale `provider.registration.stale` when recheck fails; otherwise consistent catalog and run binding |
| gated `run.request` vs `run.annotate` / `run.evidence.add` | annotation first; transition first | transition CAS unaffected by annotation; no stale invalidation from annotation |
| gated `run.request` vs concurrent transition | first transition wins | second errors `state.stale_version` with journal attempt, no state advance |
| `run.label` vs `run.request` final transition | label commits first; transition commits first | label durable when committed while `active`; terminal re-read rejects with `label.changed` journal (`run.lifecycle.terminal`) when transition committed first |
| `run.label` vs `run.terminate` | label commits first; terminate commits first | label durable when committed while `active`; terminal re-read rejects with `label.changed` journal (`run.lifecycle.terminal`) when termination committed first |
| `run.label` terminal rejection commit fault | journal present; journal absent after commit I/O failure | fresh-connection verification per [persistence.md](persistence.md) § Commit outcome verification; no false `completed` claim (I35, I44) |
| `provider.disable` with `ack_token` vs active-set change | token issued; run created/terminated before authorize | `catalog.ack_token.stale` on digest or `config_revision` mismatch |

### Cursor and disable-ack integrity (black-box)

Required black-box scenarios prove integration-owned HMAC protection for cursor v1 and `provider.disable` final-page `ack_token` values ([cli-contract.md](cli-contract.md) § Collection pagination and cursor v1; [persistence.md](persistence.md) § Integration integrity key). Tests invoke production CLI only; harnesses **MAY** mutate opaque wire strings or SQLite metadata through `LOOP_ENGINE_HOME` setup but **MUST NOT** inject product fault-injection branches.

| Scenario | Setup | Expected outcome |
|---|---|---|
| Tampered cursor MAC | Valid `next_cursor` / `--warning-cursor` with one payload byte or `mac` nibble flipped | `cursor.invalid`; no catalog mutation |
| Skipped warning page | Multi-page disable warning flow: use page-1 `next_cursor` to call `--allow-active-runs` without final-page token, or jump to a forged final-page cursor | `catalog.ack_token.invalid` or `cursor.invalid`; registration remains enabled |
| Edited `last_key` with preserved outer JSON | Re-encode cursor JSON after advancing `last_key` without re-minting MAC | `cursor.invalid` |
| Tampered disable `ack_token` MAC | Valid final-page token with flipped MAC or payload field | `catalog.ack_token.invalid`; no tombstone |
| Missing / wrong-length `integrity_key` row | Harness seeds `integration_metadata` absent, empty, or not 32 bytes before CLI open | `persistence.failed` at open; no application dispatch |
| Corrupted `integrity_key` bytes | Harness replaces key bytes with `randomblob(32)` after tokens were minted under prior key | Previously minted cursors/tokens fail MAC verify (`cursor.invalid` / `catalog.ack_token.invalid`); same-user tampering is store corruption, not authorization |
| Completed traversal binding | Full multi-page warning flow through production CLI | Final page emits `ack_token`; authorized disable succeeds once; token reuse after disable → `catalog.ack_token.stale` |

Integrity key material **MUST NOT** appear in CLI stdout/stderr, operational trace, or export artifacts in any scenario above.

Atomicity fixtures inject SQLite abort or corruption through test harness setup only; production code must not expose fault-injection branches ([persistence.md](persistence.md) § Test-only fault injection).

### Provider execution and authoring

Cover explicit invocation visibility, machine-local stable registration, configured executable/arguments/working directory/configurable timeout, caller-CWD independence, no automatic discovery/manifest import, tombstoned/missing registration, program unavailable, unsupported protocol major, timeout, non-zero exit, signal/crash, bounded raw stdout/stderr retention in operational trace, malformed output, tagged verdict/incompatibility/error results, provider evidence persistence/validation, oversized selected context rejection without truncation/invocation, one batched exact verdict set, every narrow provider operation, and boundary conformance diagnostics without gate-correctness claim.

### Configuration and run inputs

Cover global/project CLI defaults without provider rebinding, malformed configuration, handles unique among enabled registrations resolving immutable IDs, tombstone/restore by ID with free handle, zero-input workflows skipping value validation, required/invalid input rejection, graph input-independence with separate-registration alternate topology, provider-free input inspection/non-secret policy, input immutability, external-resource move with provider remap versus restore/new-run recovery, stable listable evidence IDs, missing evidence-ID rejection, self-contained/input-relative locator handoff from another CWD, optional non-unique run labels, active-only label mutation, active-default/terminal-inclusive listing, append-only evidence after terminal lifecycle, absence of individual-run deletion/compaction, and stored graph stability after provider graph changes.

### Model-based black-box testing

Generative model-based testing supplements deterministic facet-matrix suites and never substitutes for them. When adopted, executable provider fixtures may generate small graphs and event sequences. Run them through CLI and compare authoritative current state and journal facts to smaller independent reference model. Preserve seed and reproduction artifacts on failure.

## Audit export contracts

Normative `run.export` ownership, artifact set (`manifest.json`, `state.json`, `journal.jsonl`), deterministic ordering, manifest `sha256:` digests, output-directory collision/permission rules, sibling-staging atomic publication and cleanup, schema-version policy, structured CLI `data.export` shape, and no-import guarantee are defined in [export-contract.md](export-contract.md).

Required black-box scenarios prove:

- export from active, final, and terminated runs without provider subprocesses;
- `export.target.not_empty` when the output directory is non-empty and no overwrite;
- `export.target.invalid` for unusable output paths;
- SQLite row inventory and logical authority unchanged after export and after failed export cleanup;
- `journal.jsonl` line order by ascending `sequence` and provider/gate/evidence observations preserved in journal lines;
- `state.json` evidence inventory sorted by `(created_at, evidence_id)` with locators copied verbatim and never dereferenced;
- manifest payload hashes match on-disk bytes;
- no import/restore/replay command exists;
- interrupted export before staging-directory rename leaves `<DIR>` absent or empty, removes this invocation's staging directory, and allows immediate retry without `export.target.not_empty`;
- crash or I/O fault after rename but before parent-directory `fsync`: a fresh process treats a manifest whose payload hashes verify as complete and does not roll back `<DIR>` because the prior CLI exit was abnormal or `resource.exhausted`;
- crash or I/O fault during staging writes leaves only an orphan staging directory removable per [export-contract.md](export-contract.md) § Orphan staging cleanup, with `<DIR>` still absent or empty and unrelated sibling files untouched;
- concurrent export to the same `<DIR>`: exactly one completed export; the loser observes `export.target.not_empty` without corrupting the winner's artifact set;
- post-rename payload/hash mismatch with no valid manifest: cleanup removes only failed-export artifact content and does not delete unrelated user files.

Structured mode **MUST NOT** emit artifact bytes on stdout. Export directory preservation after failure applies only when a fresh-process manifest/hash verification succeeds; pre-rename failures **MUST** leave `<DIR>` absent or empty.

## Operational trace contracts

Normative JSONL v1 categories, field shapes, budget behavior, driver/parse rules, late sink-failure truthfulness, and Unix `SIGXFSZ`/`RLIMIT_FSIZE` injection contract are defined in [operational-trace.md](operational-trace.md). Required black-box scenarios parse real per-invocation JSONL trace and verify stable semantic categories rather than exact full-log snapshots.

Every dispatched operation proves:

- one secure trace file associated with invocation;
- public request ID matches trace identity;
- operation start and finish/error boundary;
- complete bounded request and outcome payloads;
- completed/rejected/error classification consistent with CLI envelope.

Provider-dependent scenarios additionally prove provider start, configured invocation facts, complete bounded protocol payloads/stdout/stderr, and provider finish/failure. Mutation scenarios prove transaction intent, applicable version check, and commit/rollback. Event scenarios prove enough state/transition/gate decision context to diagnose acceptance or denial.

Trace-initialization failure must prove no operation dispatch, provider marker, or persistence mutation. Crash scenario uses real blocking provider, terminates CLI process, and verifies flushed pre-effect markers reveal last observed phase without requiring impossible completion record. Rotation scenarios prove configured count/byte bounds, preservation of open trace, per-invocation separation, and safe concurrent processes.

Architecture/build checks prevent alternate provider/persistence/dispatch paths, but no test counts logging calls or requires per-function attributes. Selective compile-fail or mutation canaries may supplement proof; CLI trace behavior remains authority.

## Supported platform scope

Normative OS/architecture matrix, permission semantics, process termination, path rules, and unsupported-platform policy live in [technology.md](technology.md) § Supported platforms. Testing mirrors only scope that differs:

- required E2E, provider-fixture, migration, atomicity, and trace suites **MUST** pass on supported macOS and glibc Linux before release, with final acceptance recording named macOS and Linux results;
- provider fixtures are Rust executables built for the host triple under test; Unix shell-script providers appear only in scenarios that explicitly exercise shebang/script behavior;
- trace `RLIMIT_FSIZE` / `SIGXFSZ` late-sink injection cases (Cases A and B in [operational-trace.md](operational-trace.md#deterministic-unix-sigxfsz--rlimit_fsize-e2e-contract)) run only on supported macOS and Linux hosts via external wrapper with no production test branch;
- isolation harnesses set `LOOP_ENGINE_HOME` to temporary roots and never rely on Windows path or permission semantics.

## Isolation requirements

Each scenario uses:

- isolated temporary project;
- isolated temporary configuration home;
- private state/journal store;
- private rotating operational-trace directory;
- controlled executable provider;
- no caller machine configuration;
- no shared mutable fixture;
- no test-order dependency;
- fresh CLI processes across persistent operations.

Network is disabled unless scenario explicitly tests provider network policy. Ordinary behavioral assertions use CLI output rather than direct database queries. Direct fixture construction is permitted for migration/corruption and narrowly for schema-valid prerequisite state whose owning operation is not yet exposed. Such setup uses production schema, is never behavioral evidence for operation that would create state, and must be repeated through production CLI after owning operation exposes.

Required scenarios cannot be ignored, quarantined, or accepted as known failures.

## Regression policy

Every behavioral defect fix adds or identifies CLI scenario that fails against faulty behavior and passes after correction. Existing coverage counts only when failure can be demonstrated.

Generated-test failures preserve seed, project directory, provider fixture/version, invocation transcript, stdout, stderr, and when applicable the export artifact directory (`manifest.json`, `state.json`, `journal.jsonl`) produced by `run.export`.
