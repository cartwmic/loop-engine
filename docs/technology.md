# Loop Engine Technology Direction

**Status:** Rust control plane, three-crate Clean Architecture, code-only executable providers, per-run graph snapshots, and authoritative state plus immutable journal are settled. Individual libraries, configuration format, provider protocol details, platforms, and packaging remain recommendations or open decisions.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Code architecture](architecture.md)
- [Testing doctrine](testing.md)
- [Interaction storyboards](ux-storyboards.md)

## Runtime direction

Recommended baseline:

- Rust edition 2024 control plane.
- Synchronous execution.
- Three product crates: core, integrations, and CLI.
- One native CLI executable.
- No mandatory daemon or external database.
- Workflow providers as external executables over versioned subprocess protocol.

Single-binary packaging applies only to control plane and remains candidate invariant. Workflow providers may require shells, language runtimes, binaries, services, or network resources.

## Recommended component stack

| Concern | Direction | Status |
|---|---|---|
| CLI parsing | `clap` derive | Recommended |
| Provider protocol | Stable-major JSON over subprocess stdio | Settled direction |
| Structured CLI output | Versioned JSON | Strong direction |
| Engine configuration | TOML with typed global/project precedence | Open |
| Embedded persistence | `rusqlite` with bundled SQLite | Strong direction |
| Database migrations | `rusqlite_migration` | Recommended if SQLite selected |
| Core errors | `thiserror` | Recommended |
| CLI diagnostics | `miette` | Recommended |
| Timestamps | `jiff` | Recommended |
| Identifiers | UUID v7 through `uuid` | Recommended |
| Provider/file digests | SHA-256 through `sha2` | Recommended for audit identity, not replay |
| Diagnostics | `tracing` with always-on per-invocation JSONL file layer | Strong direction |
| Protocol/schema generation | `schemars` | Recommended |
| Graph validation | Direct validation plus `petgraph` where algorithms justify dependency | Candidate |

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

Provider boundary remains language-neutral and out-of-process. No dynamic-library ABI is planned.

Immutable machine-local registration ID is logical workflow identity; mutable human handle is unique among enabled registrations and resolves to ID. Same executable may back several separately configured registrations. Active run resolves current executable, arguments, and working directory through stored ID. Project defaults cannot rebind it. Executable path and observed digest are best-effort invocation/audit facts rather than workflow identity or proof of interpreted dependencies/environment.

Provider protocol has five narrow operations:

- input-free description of complete graph, optional input declarations, static guidance, and live-guidance capability;
- value-only candidate-input validation without topology output;
- required-gate evaluation returning complete verdicts/evidence, explicit incompatibility, or evaluation error;
- explicitly requested advisory/live guidance;
- compatibility check against active stored graph.

Input validation is skipped when description declares no inputs and caller supplies none. When both creation operations run, they use same resolved registration and observed executable digest when available; detected executable change errors, but digest does not cover interpreted dependencies/environment. Provider must never receive protocol authority to set current state directly.

Invocation requirements:

- executable plus argument vector, never implicit shell interpolation;
- explicit executable, argument vector, and working directory;
- no caller-CWD inheritance;
- no project scanning, import manifest, package registry, installer, or automatic discovery;
- timeout configurable per registration or invocation;
- bounded stdout/stderr retained without redaction in rotating per-invocation operational trace;
- versioned structured result;
- unavailable/crashed/malformed provider fails closed for authoritative operations;
- provider identity and available digest/version recorded;
- all bounded engine/provider payloads retained in operational trace while inherited full environment remains excluded.

A provider itself may be shell script and may execute arbitrary code. Engine does not sandbox it. Configuring explicit locator authorizes execution with caller permissions; MVP adds no separate trust database or approval ceremony.

## Operational diagnostics direction

Use standard `tracing` ecosystem with one structured JSONL file layer initialized by CLI before operation dispatch. One CLI invocation owns one request ID, one trace file, and its flush lifetime. Trace initialization or current-user-only permission failure stops invocation before dispatch.

Instrumentation stays at three choke points:

- operation dispatcher logs operation/request identity and complete bounded request/outcome payloads;
- provider subprocess integration logs invocation facts, complete bounded protocol payloads/stdout/stderr, and execution result;
- persistence integration logs transaction intent, applicable version check, and commit/rollback.

Add explicit internal events only for consequential decisions not explained by those boundaries. Do not thread trace context through every helper, require per-function attributes, add custom compiler plugin, or scan source for logging-call counts. Crate visibility and architecture checks prevent bypass; production CLI E2Es parse real trace files.

Trace rotates by configurable file-count and total-byte limits and never removes open file. It is diagnostic storage rather than state/journal authority. Exact event schema, default location/limits, and writer implementation remain implementation decisions. Full retained payloads can contain sensitive caller/provider data; no encryption or redaction is promised.

## Graph validation direction

Provider output first deserializes into protocol DTO, then converts into validated core graph.

Validation stages:

1. Protocol framing and version checks.
2. Structural DTO validation.
3. Core semantic checks for initial/final states, final-state sink rule, at most one transition per `(state,event)`, identifiers, transition targets, gate references, static-guidance/live-guidance-capability declarations, and supported semantics.

Invalid provider graph is operation error and prevents run creation. Engine computes graph-revision identity from full canonical validated projection, including topology, gates, input declarations, static guidance, and live-guidance capability. Stored graph is fixed for run and graph identity remains distinct from stable logical workflow identity. Latest provider graph is not consulted during active-run transition resolution.

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

Candidate topology:

- machine-local provider registrations and run catalog, operational state, graph snapshots, and journal in bundled SQLite;
- global/project configuration limited to CLI defaults and registration references;
- read-only stable JSON state export;
- read-only stable JSONL journal export;
- stable run-scoped evidence IDs and bounded records exactly as submitted by caller/provider;
- provider-free evidence inventory;
- external evidence locators retained as self-contained or provider-defined input-relative references without automatic dereference/copying.

If selected, bundled SQLite supplies transactions, crash recovery, cross-process writer locking, migrations, and journal queries without service. Public UX assumes one current caller per run and exposes no optimistic revision token.

Still open:

- exact machine-local database location;
- journal correction presentation;
- export canonicalization.

MVP performs no silent journal compaction and accepts unbounded local growth.

Read-only JSON/JSONL export supports inspection, not run import/restore/mobility, and is not competing authority. If SQLite is selected, writing SQLite and JSONL as dual authorities is rejected.

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
- provider fixtures in shell and compiled form;
- `proptest` for optional pure and black-box generated cases;
- temporary project directories and real selected persistence stores;
- Cargo formatting, Clippy, and dependency/advisory tooling;
- optional non-shipping Rust `xtask` for canonical gate and hooks;
- generic versioned semantic-judge executable contract with focused documentation, observability, architecture/tenet/KISS, and behavioral-evidence rubrics.

Exact test libraries remain implementation choices; [testing.md](testing.md) defines required behavior independent of harness library.

## Distribution and compatibility

Still unresolved:

- macOS/Linux versus macOS/Linux/Windows support;
- public minimum supported Rust version;
- support duration for superseded provider-protocol majors;
- journal/state database migration policy;
- structured CLI and export compatibility policy;
- release packaging tool;
- installer formats;
- license.

Pin development toolchain for reproducibility. Declare public MSRV only before first stable release.
