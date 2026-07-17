# Loop Engine Testing Doctrine

**Status:** E2E authority, facet coverage, runtime operation/trace proof, executable-provider coverage, no-mock policy, and per-commit semantic judgment are settled. Exact hook/remote implementation remains candidate.

Related documents:

- [Product intent](intent.md)
- [Core tenets](tenets.md)
- [System invariants](invariants.md)
- [Reference workflow](reference-workflow.md)
- [Code architecture](architecture.md)
- [Technology direction](technology.md)
- [Interaction storyboards](ux-storyboards.md)

## Authoritative claim

Tests cannot prove total absence of defects. Enforced claim is narrower:

> Every declared engine operation is reachable through a production driver, is observed executing in passing black-box scenarios against real providers and persistence, satisfies applicable behavioral facets, and preserves all known regression contracts.

CLI is the only current production driver.

## Behavioral authority

Only black-box production-driver tests satisfy behavioral acceptance. Tests invoke built CLI as separate process and observe documented outputs, exit codes, provider invocations, persisted state, and later CLI queries.

Lower-level unit, integration, adapter, or property tests cannot substitute for missing CLI coverage.

Pure core property tests are permitted as supplemental combinatorial defense. They must remain free of mocks and do not count toward operation completeness.

## No-mock policy

Required behavioral tests use:

- production CLI binary;
- selected production transactional persistence integration (SQLite fixture when selected);
- production executable-provider integration;
- controlled provider scripts/binaries implementing real protocol;
- production configuration and JSON protocol parsing.

Mock frameworks and mock-based behavioral tests are prohibited.

Temporary filesystems, executable provider fixtures, legacy databases, malformed protocol responses, deliberate corruption, and independent reference models are real test inputs rather than replacements for product behavior.

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

Every structured CLI outcome after dispatch includes stable operation ID and one of three semantic outcome classes:

- completed operation;
- domain rejection;
- operation error.

Domain rejection covers stored-graph/lifecycle denial, failed gate, invalid caller input/evidence selection, unsupported guidance, and explicit provider-declared capability incompatibility. Operation error covers invalid provider graph and, for provider-dependent requests, tombstoned/missing registration/executable, unsupported protocol major, creation drift, provider execution/evaluation/protocol/evidence failure, stale workflow-state/lifecycle version, and persistence failure. Successful explicit compatibility check completes with non-latching per-run/per-capability findings. Detailed reason codes preserve recovery paths without adding top-level classes.

Structured mode emits exactly one outcome envelope on stdout after dispatch. Always-on JSONL trace is written to separate file and never mixed into stdout. Stderr is reserved for rich pre-dispatch failure or inability to construct envelope.

Illustrative shape:

```json
{
  "operation": "run.request",
  "request_id": "01J...",
  "trace": "/machine-local/logs/01J....jsonl",
  "outcome": "rejected",
  "data": {},
  "diagnostics": []
}
```

Exact schema remains to be designed and versioned.

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

Lifecycle ownership is distributed, not repeated wholesale by every lifecycle-aware command: list/show/terminate own lifecycle visibility and terminal-state family; evidence/annotation own terminal append allowance; label/request/guidance/compatibility own their terminal rejection. Facet inventory must assign every family member to at least one exposure and each operation must close its applicable slice before exposure. Names in operation facet inventories must match this table exactly. No lower-level test can waive an assigned facet.

## Required reference acceptance

The [reference software-change workflow](reference-workflow.md) is mandatory black-box acceptance case. Every behavior listed in its required acceptance section must have runtime-observed production CLI coverage. Tests may combine behaviors but cannot substitute lower-level proof or omit software-specific revision paths.

## Cross-operation E2E families

Operation facets are minimum coverage, not complete suite.

### Workflow semantics

Cover linear transitions, cycles, explicit review-revision paths, completed self-loops reporting unchanged state, zero/one/multiple final states, initial-final run, final-state outgoing-transition rejection, unknown events, ambiguous emitted graphs, multiple gates, and rejection preserving state.

### Provider graph creation

Cover input-free graph description, value-only input validation returning no topology, detected description/validation observed-executable drift error and interpreted-dependency limitation, valid graph emission, malformed JSON, explicit graph check completing with invalid finding and run creation treating same invalid graph as operation error, compatible same-major additions, unsupported protocol major, immutable registration-ID workflow identity, full-projection digest including guidance/input/live-guidance-capability declarations, missing initial/target state, duplicate identifiers, duplicate `(state,event)`, unsupported semantics, provider crash, static-guidance/no-guidance/live-capability declaration, and graph snapshot persistence.

### Provider drift

Cover machine-local stable registration ID, caller-CWD-independent resolution, handle rename preserving ID, disable releasing handle, explicit restore by ID with free handle, handle reuse not capturing stranded runs, registration changing executable/arguments/working directory during active run, gate-attempt locator/digest journaling, stored projection unchanged, provider honoring stored declarations or reporting incompatibility, non-latching per-capability report with mixed findings, unsupported selected request rejecting, and supported/gate-free events remaining usable.

### Stateful sequences

Cover realistic multi-command histories through separate processes, including provider registration by unique handle/stable ID, create with inspectable non-secret inputs and optional label, machine-wide active listing from another working directory, label change while active, provider-free current-work show/full-graph inspection/history/evidence inventory, stored live-guidance capability, explicitly requested live guidance plus journaling of completed/unsupported/lifecycle/error requests, independent evidence/note append, cold-session inventory with empty-default caller selection and guidance recommendations, event request with inline evidence, reject, transition, cycle, terminate with note, terminal annotation, and later continuation of active runs. Verify normal responses report current state, state-change status, and next requestable events.

### Persistence and journal

Cover restart, authoritative-state loading, journal ordering, atomicity of every mutation and attempt-only journal/evidence append, unknown-event and terminal-denial journaling after run lookup, no run journal for rejected creation, distinct committed-status reporting for inline evidence, selected associations, and provider evidence on applicable attempts, provider timeout/crash journaling, abrupt process death with explicitly limited audit guarantee, migration, corruption, export, interrupted processes, and history inspection before manual retry.

Do not require replay or historical state reconstruction.

### Mutation safety

Verify caller-facing commands require no revision token, claim, lease, or idempotency key. Accidental overlap must not corrupt state/journal; provider verdict evaluated against stale workflow-state/lifecycle version must error without transition, while concurrent label/note/evidence append does not invalidate it. Intentional concurrent same-run collaboration remains outside accepted workflow.

### Provider execution and authoring

Cover explicit invocation visibility, machine-local stable registration, configured executable/arguments/working directory/configurable timeout, caller-CWD independence, no automatic discovery/manifest import, tombstoned/missing registration, program unavailable, unsupported protocol major, timeout, non-zero exit, signal/crash, bounded raw stdout/stderr retention in operational trace, malformed output, tagged verdict/incompatibility/error results, provider evidence persistence/validation, oversized selected context rejection without truncation/invocation, one batched exact verdict set, every narrow provider operation, and boundary conformance diagnostics without gate-correctness claim.

### Configuration and run inputs

Cover global/project CLI defaults without provider rebinding, malformed configuration, handles unique among enabled registrations resolving immutable IDs, tombstone/restore by ID with free handle, zero-input workflows skipping value validation, required/invalid input rejection, graph input-independence with separate-registration alternate topology, provider-free input inspection/non-secret policy, input immutability, external-resource move with provider remap versus restore/new-run recovery, stable listable evidence IDs, missing evidence-ID rejection, self-contained/input-relative locator handoff from another CWD, optional non-unique run labels, active-only label mutation, active-default/terminal-inclusive listing, append-only evidence after terminal lifecycle, absence of individual-run deletion/compaction, and stored graph stability after provider graph changes.

### Model-based black-box testing

Executable provider fixtures may generate small graphs and event sequences. Run them through CLI and compare authoritative current state and journal facts to smaller independent reference model. Preserve seed and reproduction artifacts on failure.

## Operational trace contracts

Required black-box scenarios parse real per-invocation JSONL trace and verify stable semantic categories rather than exact full-log snapshots.

Every dispatched operation proves:

- one secure trace file associated with invocation;
- public request ID matches trace identity;
- operation start and finish/error boundary;
- complete bounded request and outcome payloads;
- completed/rejected/error classification consistent with CLI envelope.

Provider-dependent scenarios additionally prove provider start, configured invocation facts, complete bounded protocol payloads/stdout/stderr, and provider finish/failure. Mutation scenarios prove transaction intent, applicable version check, and commit/rollback. Event scenarios prove enough state/transition/gate decision context to diagnose acceptance or denial.

Trace-initialization failure must prove no operation dispatch, provider marker, or persistence mutation. Crash scenario uses real blocking provider, terminates CLI process, and verifies flushed pre-effect markers reveal last observed phase without requiring impossible completion record. Rotation scenarios prove configured count/byte bounds, preservation of open trace, per-invocation separation, and safe concurrent processes.

Architecture/build checks prevent alternate provider/persistence/dispatch paths, but no test counts logging calls or requires per-function attributes. Selective compile-fail or mutation canaries may supplement proof; CLI trace behavior remains authority.

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

Generated-test failures preserve seed, project directory, provider fixture/version, invocation transcript, stdout, stderr, state export, and journal export.

Suite runtime receives explicit budget. When growth exceeds budget, shard or optimize harness before weakening contracts.

## Canonical quality gate

Complete revision gate includes:

- formatting;
- compilation;
- Clippy with warnings denied;
- architecture dependency checks;
- optional pure property tests;
- provider-protocol same-major and conformance fixtures;
- state/journal migration and atomicity fixtures;
- complete CLI E2E suite;
- operation/driver/E2E/trace-catalog coverage equality;
- focused semantic-judge rubrics for documentation impact, observability, architecture/tenets/KISS, and behavioral evidence;
- dependency, license, and advisory checks.

Behavioral authority remains with CLI E2Es even when supplemental checks run.

## Git enforcement direction

Git hooks can enforce cooperative local workflow but cannot be unbypassable on user-controlled machine.

Settled semantic policy:

- one generic versioned judge-executable contract supports focused rubrics for documentation impact, observability, architecture/tenet adherence including KISS, and behavioral evidence;
- each commit is judged independently from exact parent-to-commit diff and resulting tree;
- parent revision's rubric judges candidate revision, so changed rubric applies only to following commit; initial foundation commit and first publication use explicit owner-approved bootstrap exception and become parent rubric thereafter;
- determinate local failure blocks commit; unavailable/indeterminate local result warns and permits commit;
- pre-push and authoritative remote gate fail closed on failed, unavailable, or indeterminate judgment for any commit;
- semantic judges receive deterministic build/test/check evidence and must cite changed lines/rubric rules rather than invent compilation or test claims;
- deterministic documentation, architecture, and quality checks remain separate from semantic judgment.

Candidate local mechanism:

- version hooks under `.githooks/`;
- fast deterministic checks plus semantic-judge attempt against exact staged content and parent rubric at pre-commit;
- full pre-push gate judges every unpublished commit against its parent in temporary detached worktree;
- no duplicated gate logic between hooks and CI;
- non-shipping Rust `xtask` installs hooks and runs canonical gate.

Candidate authoritative mechanism after remote exists:

- protect `main` from direct writes;
- require branch current with `main`;
- require canonical gate before merge;
- make releases depend on same gate;
- prevent bypass where hosting platform supports it.

Server-side controls are authority. Local hooks provide earlier feedback and accidental-regression protection.
