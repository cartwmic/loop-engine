# Loop Engine Technology Direction

**Status:** Rust control plane, three-crate Clean Architecture, code-only executable providers, per-run graph snapshots, authoritative state plus immutable journal, bundled SQLite persistence, machine-local configuration, bounded CLI resources, operational trace JSONL, canonical graph encoding, and read-only audit export are settled. Individual libraries beyond approved dependencies and release packaging beyond Cargo-built binaries remain recommendations or open decisions. Provider protocol transport, machine-local paths and project defaults, CLI bounds and pagination, SQLite semantics, graph projection, audit export, and evidence wire schemas are defined by their respective contracts.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Interaction storyboards](ux-storyboards.md)
- [Machine-local configuration](configuration.md)
- [SQLite persistence policy](persistence.md)
- [Graph projection and canonical encoding](graph-projection.md)
- [Export contract](export-contract.md)

## Runtime direction

Recommended baseline:

- Rust edition 2024 control plane.
- Synchronous execution.
- Three product crates: core, integrations, and CLI.
- One native CLI executable.
- No mandatory daemon or external database.
- Workflow providers as external executables over versioned subprocess protocol.

Single-binary packaging applies only to control plane and is settled by C1. Workflow providers may require shells, language runtimes, binaries, services, or network resources.

## Recommended component stack

| Concern | Direction | Status |
|---|---|---|
| CLI parsing | `clap` derive | Settled |
| Provider protocol | Stable-major JSON over subprocess stdio (v1 transport frozen) | Settled |
| Structured CLI output | Versioned JSON | Strong direction |
| Engine configuration | TOML with typed global/project precedence and nearest-ancestor `.loop-engine.toml` discovery | Settled — [configuration.md](configuration.md) |
| Embedded persistence | `rusqlite` with bundled SQLite | Settled |
| Database migrations | `rusqlite_migration` | Settled |
| Core errors | `thiserror` | Settled |
| CLI diagnostics | `miette` | Settled |
| Timestamps | `jiff` | Settled |
| Identifiers | UUID v7 through `uuid` | Settled |
| Provider/file digests | SHA-256 through `sha2` | Settled |
| Diagnostics | `tracing` with always-on per-invocation JSONL file layer | Settled |
| Protocol/schema generation | `schemars` | Settled |
| Graph validation | Direct validation plus `petgraph` where algorithms justify dependency | Settled |

Dependency versions should be pinned through Cargo.lock and assessed for maintenance, licensing, and advisories before adoption.

## Workflow authoring

Workflow authoring is code-only. No YAML/JSON workflow DSL is planned.

Any executable language may implement provider protocol. Provider is cohesive source for:

- graph declarations;
- state/event/transition/gate identifiers;
- optional immutable run-input declarations;
- static guidance and live-guidance capability;
- gate evaluation;
- optional dynamic guidance and gate-produced evidence.

At run creation provider emits normalized JSON graph projection. Engine validates and stores it. JSON is protocol representation, not hand-authored workflow source.

Run inputs are optional provider-declared, provider-free inspectable, non-secret values validated and frozen at creation. MVP graph projection is input-independent; alternate topology uses separate registration. Evidence accumulates append-only. Core supplies no artifact, workspace, repository, or mutable-variable semantics.

MVP authoring support provides protocol schemas, examples, actionable diagnostics, and engine-run conformance checks. Official language SDK is not required. Future SDKs, generators, single-file templates, and GUI tools must implement or generate same executable provider contract.

## Provider subprocess direction

Normative transport contract: [provider-protocol-v1.md](provider-protocol-v1.md).

Provider boundary remains language-neutral and out-of-process. No dynamic-library ABI is planned.

Immutable machine-local registration ID is logical workflow identity; mutable human handle is unique among enabled registrations and resolves to ID. Same executable may back several separately configured registrations. Active run resolves current executable, arguments, and working directory through stored ID. Project defaults cannot rebind it. Executable path and observed digest are best-effort invocation/audit facts rather than workflow identity or proof of interpreted dependencies/environment.

Provider protocol has five narrow operations:

- input-free description of complete graph, optional input declarations, static guidance, and live-guidance capability;
- value-only candidate-input validation without topology output;
- required-gate evaluation returning complete verdicts/evidence, explicit incompatibility, or evaluation error;
- explicitly requested advisory/live guidance;
- compatibility check against active stored graph.

Input validation is skipped when description declares no inputs and caller supplies none. When both creation operations run, they use same resolved registration and observed executable digest when available; detected executable change errors, but digest does not cover interpreted dependencies/environment. Provider must never receive protocol authority to set current state directly.

Invocation requirements (see [provider-protocol-v1.md](provider-protocol-v1.md) for byte-level rules):

- one fresh OS process per role invocation; no persistent stdio session;
- literal `executable` plus verbatim `argv` argument vector; never implicit shell interpolation;
- absolute `working_directory` independent of caller CWD;
- environment inherited unchanged from engine process; registration stores no environment overrides; inherited environment is never copied into operational trace;
- core operation resolves registration exactly once and passes immutable `registration` object; invoker never queries catalog during provider execution;
- one UTF-8 JSON request envelope on stdin, then EOF; exactly one UTF-8 JSON result envelope on stdout;
- `protocol_major` 1 with same-major unknown-field ignore; unsupported major errors;
- timeout configurable per registration (`timeout_seconds`, default `provider_timeout_seconds_default` per [cli-contract.md](cli-contract.md#resource-bounds)); on supported Unix platforms timeout kills the provider process group (`SIGTERM`, 5-second grace per [provider-protocol-v1.md](provider-protocol-v1.md), then `SIGKILL`);
- bounded stdout/stderr retained without redaction in rotating per-invocation operational trace;
- unavailable/crashed/malformed/timed-out provider fails closed for authoritative operations;
- provider identity and available digest/version recorded;
- no automatic retries (I44); provider results never set engine state directly.

A provider itself may be shell script and may execute arbitrary code. Engine does not sandbox it. Configuring explicit locator authorizes execution with caller permissions; MVP adds no separate trust database or approval ceremony.

## Operational diagnostics direction

Use standard `tracing` ecosystem with one structured JSONL file layer initialized by CLI before operation dispatch. One CLI invocation owns one request ID, one trace file, and its flush lifetime. Trace initialization or current-user-only permission failure stops invocation before dispatch. On supported Unix platforms, trace directories use mode `0700` and trace files mode `0600`.

Instrumentation stays at three choke points:

- operation dispatcher logs operation/request identity and complete bounded request/outcome payloads;
- provider subprocess integration logs invocation facts, complete bounded protocol payloads/stdout/stderr, and execution result;
- persistence integration logs transaction intent, applicable version check, and commit/rollback.

Add explicit internal events only for consequential decisions not explained by those boundaries. Do not thread trace context through every helper, require per-function attributes, add custom compiler plugin, or scan source for logging-call counts. Crate visibility and architecture checks prevent bypass; production CLI E2Es parse real trace files.

Trace rotates by configurable file-count and total-byte limits (`trace_retained_files_max`, `trace_directory_budget_bytes`; [cli-contract.md](cli-contract.md#resource-bounds)) and never removes an open file. Cross-process rotation counts actual encoded bytes plus only unused reservation remainder; per-invocation reservations (`trace_init_reservation_bytes`, `trace_provider_call_reservation_bytes`, `provider_calls_per_paged_invocation_max`) are defined in [cli-contract.md](cli-contract.md#operational-trace-budgets-cross-process). Trace is diagnostic storage rather than state/journal authority. JSONL v1 event categories, field shapes, permissions, flush lifecycle, late sink-failure truthfulness, and E2E injection contract are defined in [operational-trace.md](operational-trace.md). Full retained payloads can contain sensitive caller/provider data; no encryption or redaction is promised.

## Graph validation direction

Provider output first deserializes into protocol DTO, then converts into validated core graph.

Validation stages:

1. Protocol framing and version checks.
2. Structural DTO validation.
3. Core semantic checks for initial/final states, final-state sink rule, at most one transition per `(state,event)`, identifiers, transition targets, gate references, static-guidance/live-guidance-capability declarations, and supported semantics.

Invalid provider graph is operation error and prevents run creation. Engine computes `graph_revision` from canonical integration DTO bytes after wire mapping and core semantic validation, including topology, gates, input declarations, static guidance, live-guidance capability, and retained provider metadata ([graph-projection.md](graph-projection.md)). Canonical encoding is integration-owned; core exposes digest-relevant semantic fields without Serde or JSON serialization. Stored canonical snapshot is fixed for run; `graph_revision` remains distinct from stable registration identity and from `executable_digest`. Latest provider graph is not consulted during active-run transition resolution.

## Provider drift direction

Provider implementation drift is allowed and logged without approval.

Each provider result engine can durably observe should capture available identity, digest, and self-reported version. Exact historical provider retention is not required.

Changed provider must still honor stored declarations/guidance contract or explicitly report capability incompatibility. Gate evaluation returns complete verdict set, explicit incompatibility, or evaluation error. Compatibility check is non-latching per-capability report; provider-dependent request rejects only when selected capability is unsupported. Gate-free and supported events remain usable. For provider-dependent request, missing/tombstoned executable, unsupported protocol major, crash, timeout, missing/malformed result, or malformed provider evidence is operation error. Gate-free events and provider-free operations continue without executable. Compatibility is structural and does not certify unchanged gate policy. Checks occur explicitly or during provider-dependent operations, never as hidden work in safe inspection. Incompatible runs remain inspectable, annotatable, and terminable. New graph emitted by changed provider affects new runs only; MVP offers no migration or bypass.

## Persistence direction

Settled authority model:

- current run state and lifecycle are authoritative stored records;
- validated graph snapshot is stored per run;
- activity journal is immutable, ordered, and explanatory;
- every durable run mutation and required journal entry are atomic;
- after run lookup, every syntactically valid event request appends all-or-nothing attempt/evidence record when persistence remains available;
- deterministic replay and historical re-execution are not promised.

Selected topology:

- machine-local provider registrations and run catalog, operational state, graph snapshots, and journal in bundled SQLite;
- global/project configuration limited to CLI defaults and registration references;
- read-only stable JSON state export and JSONL journal export with completion manifest per [export-contract.md](export-contract.md);
- stable run-scoped evidence IDs and bounded records exactly as submitted by caller/provider;
- provider-free evidence inventory;
- external evidence locators retained as self-contained or provider-defined input-relative references without automatic dereference/copying.

Bundled SQLite supplies transactions, crash recovery, cross-process writer locking, migrations, and journal queries without service. Public UX assumes one current caller per run and exposes no optimistic revision token.

Exact pragmas, busy timeout, migration rules, schema-version compatibility, transaction boundaries, workflow/registration CAS, catalog-mutation digest guards, rollback narratives, and export read-snapshot semantics are defined in [persistence.md](persistence.md). Exact machine-local paths (`state.db`, `traces/`, global/project TOML locations, `LOOP_ENGINE_HOME` semantics) are defined in [configuration.md](configuration.md). Export canonical encoding, ordering, manifest hashes, and schema-version policy are defined in [export-contract.md](export-contract.md).

Still open:

- journal correction presentation;

MVP performs no silent journal compaction and accepts unbounded local growth.

Read-only JSON/JSONL export supports inspection, not run import/restore/mobility, and is not competing authority. Writing SQLite and export files as dual authorities is rejected. Normative export behavior is [export-contract.md](export-contract.md).

## Determinism boundary

Engine decision is deterministic given:

- stored graph;
- authoritative prior workflow-state/lifecycle version;
- requested event and bounded evaluation snapshot;
- observed provider gate verdicts.

Provider execution may observe changing files, tools, clocks, services, or networks and may produce different verdicts. Journal records observed result. Engine does not rerun it during resume or history inspection.

## Deliberate MVP exclusions

- Tokio or another async runtime.
- HTTP server.
- External workflow service.
- ORM.
- YAML/JSON workflow DSL.
- CEL or general expression language.
- Embedded JavaScript/runtime for authoring.
- Dynamic library plugins.
- BPMN/SCXML runtime.
- Automatic agent invocation.
- Distributed scheduler or worker queue.
- Event-sourced state reconstruction.
- Provider sandboxing or separate trust database.
- Automatic provider discovery, package registry, or installer.
- Work claims and leases.
- Caller-managed optimistic revision tokens.
- Same-run concurrent-collaboration protocol.
- Caller idempotency keys and automatic provider retries.
- Pause/resume or terminal-reopen lifecycle status.
- Individual-run deletion.
- Signed journal entries and actor key management.
- Active-run graph migration or gate bypass.
- Custom compiler plugin solely to require per-function trace instrumentation.

Each exclusion should be revisited only against concrete requirement.

## Testing and tooling

Production behavior is tested through CLI with executable provider fixtures. Expected tooling may include:

- standard Rust test harness for E2Es;
- `assert_cmd` or direct `std::process::Command` harnessing;
- Rust compiled provider fixtures (`test-support/providers/*`); Unix shell-script providers only in explicit shebang scenarios;
- `proptest` for optional pure and black-box generated cases;
- temporary project directories and selected SQLite persistence stores;
- Cargo formatting, Clippy, and dependency/advisory tooling;
- operation facet manifests under [`quality/facets/v1/`](../quality/facets/v1/), which record production CLI coverage;
- standard dependency and advisory tooling where needed for local development.

Exact test libraries remain implementation choices; [testing.md](testing.md) defines required behavior independent of harness library.

## Distribution and compatibility

Settled for MVP:

- shared crate/package version `0.1.0`;
- one native `loop-engine` binary built from source with Cargo only;
- no installer, package-manager distribution, or release-packaging tool commitment;
- no public minimum supported Rust version before first stable release;
- development toolchain pinned in `rust-toolchain.toml` (see below).

Settled for MVP:

- supported OS families: macOS and Linux (glibc) only;
- supported Rust target triples: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`;
- Windows and all other OS/architecture combinations are unsupported for MVP;
- Unix semantics for permissions, process-group termination, signals, paths, and shell-script providers without speculative cross-platform abstraction.

Settled for MVP:

- dual MIT/Apache-2.0 project license;
- canonical license texts at root `LICENSE-MIT` and `LICENSE-APACHE`;
- root `README.md` dual-license notice;
- workspace and every shipping crate use SPDX expression `license = "MIT OR Apache-2.0"`;
- workspace package metadata references root `readme = "README.md"` where applicable;
- root-excluded `test-support/providers/*` standalone fixtures use the same SPDX expression unless a fixture documents a divergent test-only policy.

Still unresolved:

- support duration for superseded provider-protocol majors.

Pin development toolchain for reproducibility. Declare public MSRV only before first stable release.

## Supported platforms

MVP supports exactly four Rust target triples on two OS families. Implementation and tests use Unix semantics directly; there is no Windows code path and no runtime compatibility shim.

### Support matrix

| OS family | Supported triples | Notes |
|---|---|---|
| macOS | `aarch64-apple-darwin`, `x86_64-apple-darwin` | Apple Silicon and Intel Mac hosts |
| Linux (glibc) | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` | Standard GNU/Linux hosts; musl-only and other libc variants are out of scope |

### Explicitly unsupported for MVP

- Windows (all triples);
- BSD, Android, iOS, WebAssembly, and embedded targets;
- 32-bit and other unlisted architectures;
- Linux targets outside the four triples above (including `*-linux-musl`).

### Unsupported-platform policy

- Building, testing, or running `loop-engine` on an unsupported target is not a supported workflow.
- If the control plane is invoked on an unsupported host, it **MUST** fail before operation dispatch with exit code `64` and a rich stderr message naming the detected target and listing supported triples. No partial feature set is offered.
- Windows support is deferred, not merely untested: it requires redesign of provider subprocess transport, filesystem layout, operational trace permissions and late-sink behavior, and provider fixtures before any Windows-targeting work proceeds.

### Unix implementation semantics

These behaviors apply on all supported platforms without alternate implementations:

| Concern | MVP behavior |
|---|---|
| File permissions | Machine-local state, trace, and other sensitive paths are current-user-only: directories `0700`, files `0600`. Permission failure before dispatch stops the invocation. |
| Provider timeout | Kill the verified provider process group: `SIGTERM`, brief grace, then `SIGKILL`. Conforming descendants must remain in that group and must not survive timeout; providers are not sandboxed against deliberate group escape. |
| Provider exit/signals | Non-zero exit, signal termination, and crash are interpreted per Unix process semantics and mapped to operation errors. |
| Paths | Unix path separators; lexical absolute normalization at registration/update; empty `LOOP_ENGINE_HOME` treated as unset; non-empty `LOOP_ENGINE_HOME` overrides machine-local roots for tests and portable use with existing-path symlink resolution or lexical identity for nonexistent roots; caller CWD is never inherited for machine-local roots or provider working directory; normative layout in [configuration.md](configuration.md). |
| Shell providers | A provider may be a shebang script invoked as the configured executable path. Engine never performs implicit shell interpolation on executable or argument vector. |

## Pinned toolchain and workspace layout

| Setting | Value |
|---|---|
| `rust-toolchain.toml` channel | `1.95.0` |
| Crate edition | `2024` |
| Workspace resolver | `3` |
| Shared `[workspace.package].version` | `0.1.0` |

Workspace members:

- `crates/loop-engine-core`
- `crates/loop-engine-integrations`
- `crates/loop-engine-cli`

Root workspace **MUST** exclude provider fixture crates. Standalone packages live outside workspace membership at:

- `test-support/providers/scenario-provider/`
- `test-support/providers/reference-provider/`

Each fixture package carries its own tracked `Cargo.lock`, is built with `cargo test --manifest-path … --locked`, and **MUST NOT** be depended on by product crates.

## Approved dependency contract

Exact crate versions and enabled features below describe current workspace dependencies and fixture bootstrap. Patch versions are exact; future additions should preserve the approved dependency policy.

### Shared workspace dependency versions

| Crate | Version | Enabled features |
|---|---|---|
| `thiserror` | `2.0.18` | — |
| `jiff` | `0.2.32` | — |
| `uuid` | `1.24.0` | `v7` |
| `sha2` | `0.11.0` | — |
| `schemars` | `1.2.1` | `derive`, `jiff02`, `uuid1` |
| `petgraph` | `0.8.3` | — |
| `serde` | `1.0.228` | `derive` |
| `serde_json` | `1.0.150` | — |
| `rusqlite` | `0.40.1` | `bundled` |
| `rusqlite_migration` | `2.6.0` | — |
| `tracing` | `0.1.44` | — |
| `tracing-subscriber` | `0.3.23` | `env-filter`, `json` |
| `tracing-appender` | `0.2.5` | — |
| `clap` | `4.6.2` | `derive` |
| `miette` | `7.6.0` | `fancy` |
| `toml` | `1.1.3` | — |
| `assert_cmd` | `2.2.2` | — |
| `proptest` | `1.11.0` | — |
| `tempfile` | `3.27.0` | — |

`miette` `fancy` is enabled only in `loop-engine-cli`, the composition root.

### `loop-engine-core`

Runtime: `thiserror`, `jiff`, `uuid`, `sha2`, `petgraph` at shared versions/features above.

Dev: `proptest`.

**MUST NOT** depend on Clap, Serde, `schemars`, SQLite, process runners, configuration parsers, or `tracing-subscriber`.

### `loop-engine-integrations`

Runtime: `loop-engine-core` (path), `rusqlite`, `rusqlite_migration`, `serde`, `serde_json`, `thiserror`, `jiff`, `uuid`, `sha2`, `schemars`, `tracing`, `toml`.

Dev: `tempfile`.

### `loop-engine-cli`

Runtime: `loop-engine-core`, `loop-engine-integrations`, `clap`, `miette`, `serde`, `serde_json`, `thiserror`, `jiff`, `tracing`, `tracing-subscriber`, `tracing-appender`.

Dev: `assert_cmd`, `tempfile`.

### Standalone provider fixtures

Provider fixtures implement real protocol subprocess providers for production CLI E2Es. They are **not** workspace members, **not** product dependencies, and **not** in-process test doubles.

#### Package layout

| Path | Binary name | Purpose |
|---|---|---|
| `test-support/providers/scenario-provider/` | `scenario-provider` | Generic configurable provider: graph/input variants, role semantics, transport/process failures, barrier, invocation ledger |
| `test-support/providers/reference-provider/` | `reference-provider` | Software-change reference workflow per [reference-workflow.md](reference-workflow.md); semantic tests owned by fixture |
| `test-support/providers/process-helpers/` | per-helper | Optional tiny Unix executables for signal/process-group/PGID cases fixtures cannot safely self-inflict; no protocol roles |

Each fixture package carries its own tracked `Cargo.lock`, builds with `cargo build --manifest-path … --locked` (or `cargo test --manifest-path … --locked`), and **MUST NOT** be depended on by product crates.

#### Language, runtime, and dependencies

- Rust edition `2024`, toolchain `1.95.0` (same pinned channel as product).
- One native executable per supported host triple when that platform is under acceptance.
- Runtime dependencies only: `serde`, `serde_json`, `schemars`, `thiserror` at shared versions/features above.
- No Python, Node, or other acceptance runtime for core E2Es. Unix shell appears only when a scenario explicitly exercises shebang/script provider configuration or when a `process-helpers/` binary is a minimal signal/PGID probe.

#### Subprocess isolation

Fixtures **MUST**:

- run as the configured provider executable in a fresh OS process per role invocation;
- speak protocol v1 on stdin/stdout only;
- keep software-domain semantics inside fixture packages (reference workflow graph, gate policies, evidence conventions).

Fixtures **MUST NOT**:

- depend on, import, link, or `include!` `loop-engine-core`, `loop-engine-integrations`, `loop-engine-cli`, or generated product schemas;
- open, read, or write authoritative `state.db`, engine `traces/`, or registration catalog;
- act as mocks, stubs, or in-process shims for product behavior.

Scenario configuration, golden vectors, and ledger files live under scenario-controlled temporary directories or each package `fixtures/` tree only.

#### Invocation ledger and barrier

`scenario-provider` exposes:

- **Invocation ledger:** append-only JSONL recording `invocation_id`, `role`, `executable`, verbatim `argv`, `working_directory`, and optional digest-mode facts for drift proofs. Path is selected through fixture argv/config under scenario control. E2E harnesses read it to prove invocation counts and empty-ledger facets; it is not engine authority.
- **Barrier:** filesystem/pipe synchronization for explicit concurrent overlap without timing-only sleeps.

#### Process-failure modes and helpers

`scenario-provider` selects deterministic failure modes from [provider-protocol-v1.md](provider-protocol-v1.md) § Transport and process failures (malformed JSON, missing/extra stdout, unsupported `protocol_major`, invalid UTF-8, oversized streams, nonzero exit, signal, timeout, drift between paired creation calls, and controllable role-specific semantic errors). Golden vectors live under `fixtures/` subtrees (`graphs/`, `inputs/`, `roles/`, `process/`).

When a case requires signal delivery, PGID verification, or orphaned-child cleanup that the fixture cannot perform safely from its own process tree, register a `test-support/providers/process-helpers/` executable instead. Helpers implement no protocol roles, import no product crates, and touch no authoritative database.

#### CI and runtime prerequisites

| Prerequisite | Requirement |
|---|---|
| Toolchain | Rust `1.95.0` from root `rust-toolchain.toml` |
| Build | `cargo build --manifest-path test-support/providers/<package>/Cargo.toml --locked` before E2E harness resolves absolute executable paths |
| Platform evidence | Fixture-owned tests and dependent CLI E2Es run on named macOS and glibc Linux acceptance hosts |
| Product coupling | Zero workspace `path` or version dependency on fixture crates |

Testing obligations and facet usage of the ledger are mirrored in [testing.md](testing.md) § Provider fixture strategy.
